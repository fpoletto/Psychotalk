use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use psychotalk_desktop::{
    engine::Engine,
    envelopes::{Envelopes, OsStore},
    server::{App, router},
};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;
fn fixture() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let store = OsStore::new(dir.path()).unwrap();
    let engine = Arc::new(
        Engine::new(
            Envelopes::new(dir.path(), store).unwrap(),
            dir.path().join("missing-tor"),
        )
        .unwrap(),
    );
    (dir, App::new(engine, "127.0.0.1:32123".into()))
}
fn req(app: &App, host: &str, origin: &str, token: &str, tab: &str, path: &str) -> Request<Body> {
    let _ = app;
    Request::builder()
        .method("POST")
        .uri(path)
        .header("host", host)
        .header("origin", origin)
        .header("content-type", "application/json")
        .header("x-psychotalk-token", token)
        .header("x-psychotalk-tab", tab)
        .body(Body::from("{}"))
        .unwrap()
}
async fn exchange(app: &App, proof: &str, tab: &str, path: &str) -> (StatusCode, Value) {
    let mut request = req(
        app,
        &app.authority,
        &format!("http://{}", app.authority),
        "",
        tab,
        path,
    );
    *request.body_mut() = Body::from(json!({"proof":proof}).to_string());
    let response = router(app.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    assert_eq!(response.headers()["cache-control"], "no-store, max-age=0");
    (
        status,
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap(),
    )
}
async fn authorize(app: &App, tab: &str) -> Value {
    let proof = app.url().split('#').nth(1).unwrap().to_string();
    let (status, value) = exchange(app, &proof, tab, "/api/authorize").await;
    assert_eq!(status, StatusCode::OK);
    value
}
#[tokio::test]
async fn rejects_foreign_origin_dns_rebinding_missing_capability_and_second_tab() {
    let (_d, app) = fixture();
    let tab = "a".repeat(64);
    let grant = authorize(&app, &tab).await;
    let token = grant["token"].as_str().unwrap();
    let host = app.authority.as_str();
    let origin = format!("http://{host}");
    for (h, o, t, expected) in [
        (host, "https://evil.example", token, StatusCode::FORBIDDEN),
        (
            "evil.example",
            origin.as_str(),
            token,
            StatusCode::FORBIDDEN,
        ),
        (host, origin.as_str(), "wrong", StatusCode::UNAUTHORIZED),
    ] {
        assert_eq!(
            router(app.clone())
                .oneshot(req(&app, h, o, t, &tab, "/api/status"))
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    let response = router(app.clone())
        .oneshot(req(&app, host, &origin, token, &tab, "/api/status"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store, max-age=0");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    assert!(
        !response
            .headers()
            .contains_key("access-control-allow-origin")
    );
    assert_eq!(
        router(app.clone())
            .oneshot(req(
                &app,
                host,
                &origin,
                token,
                &"b".repeat(64),
                "/api/status"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    app.reset();
    assert_eq!(
        router(app.clone())
            .oneshot(req(&app, host, &origin, token, &tab, "/api/status"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        exchange(&app, grant["resume"].as_str().unwrap(), &tab, "/api/resume")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let new = authorize(&app, &"b".repeat(64)).await;
    assert_ne!(grant["token"], new["token"]);
}
#[tokio::test]
async fn rejects_missing_origin_oversized_body_and_locked_command() {
    let (_d, app) = fixture();
    let tab = "d".repeat(64);
    let grant = authorize(&app, &tab).await;
    let token = grant["token"].as_str().unwrap();
    let origin = format!("http://{}", app.authority);
    for path in [
        "/api/authorize",
        "/api/resume",
        "/api/unlock",
        "/api/command",
    ] {
        let missing = Request::builder()
            .method("POST")
            .uri(path)
            .header("host", &app.authority)
            .header("x-psychotalk-token", token)
            .header("x-psychotalk-tab", &tab)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            router(app.clone()).oneshot(missing).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let foreign = req(
            &app,
            &app.authority,
            "https://evil.example",
            token,
            &tab,
            path,
        );
        assert_eq!(
            router(app.clone()).oneshot(foreign).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        let mut oversized = req(&app, &app.authority, &origin, token, &tab, path);
        *oversized.body_mut() = Body::from("x".repeat(17000));
        assert_eq!(
            router(app.clone())
                .oneshot(oversized)
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }
    assert_eq!(
        router(app.clone())
            .oneshot(req(
                &app,
                &app.authority,
                &origin,
                token,
                &tab,
                "/api/command"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn resume_is_not_an_api_key_and_rotates_the_document_without_unlocking() {
    let (_d, app) = fixture();
    let first = "a".repeat(64);
    let second = "b".repeat(64);
    let bootstrap = app.url().split('#').nth(1).unwrap().to_string();
    let grant = authorize(&app, &first).await;
    let token = grant["token"].as_str().unwrap();
    let resume = grant["resume"].as_str().unwrap();
    let origin = format!("http://{}", app.authority);
    assert_eq!(
        exchange(&app, &bootstrap, &first, "/api/authorize").await.0,
        StatusCode::UNAUTHORIZED
    );
    for proof in [resume, bootstrap.as_str()] {
        assert_eq!(
            router(app.clone())
                .oneshot(req(
                    &app,
                    &app.authority,
                    &origin,
                    proof,
                    &first,
                    "/api/status"
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        exchange(&app, "wrong", &second, "/api/resume").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        router(app.clone())
            .oneshot(req(
                &app,
                &app.authority,
                &origin,
                token,
                &first,
                "/api/status"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let (status, next) = exchange(&app, resume, &second, "/api/resume").await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(next["token"], grant["token"]);
    assert_ne!(next["resume"], grant["resume"]);
    assert!(!app.engine.status()["unlocked"].as_bool().unwrap());
    for path in [
        "/api/status",
        "/api/lock",
        "/api/heartbeat",
        "/api/unlock",
        "/api/command",
    ] {
        assert_eq!(
            router(app.clone())
                .oneshot(req(&app, &app.authority, &origin, token, &first, path))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        exchange(&app, resume, &first, "/api/resume").await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        router(app.clone())
            .oneshot(req(
                &app,
                &app.authority,
                &origin,
                next["token"].as_str().unwrap(),
                &second,
                "/api/status"
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let restarted = App::new(app.engine.clone(), app.authority.clone());
    assert_eq!(
        exchange(
            &restarted,
            next["resume"].as_str().unwrap(),
            &second,
            "/api/resume"
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}
