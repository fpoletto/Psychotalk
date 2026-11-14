//! Same-origin, capability-authenticated loopback UI. No remotely reachable control API.
use crate::{
    authorization::{Authorization, ProofKind},
    engine::Engine,
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Extension, Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use zeroize::{Zeroize, Zeroizing};
#[derive(Clone)]
pub struct App {
    pub engine: Arc<Engine>,
    pub authority: String,
    authorization: Arc<Mutex<Authorization>>,
    operations: Arc<Mutex<()>>,
}
#[derive(Clone, Copy)]
struct AccessGrant(u64);
#[derive(Clone)]
struct WindowId(String);
#[derive(Clone, Copy)]
struct CompletedCleanup;
impl App {
    pub fn new(engine: Arc<Engine>, authority: String) -> Self {
        Self {
            engine,
            authority,
            authorization: Arc::new(Mutex::new(Authorization::new(Instant::now()))),
            operations: Arc::new(Mutex::new(())),
        }
    }
    pub fn url(&self) -> String {
        format!(
            "http://{}/#{}",
            self.authority,
            self.authorization.lock().unwrap().bootstrap()
        )
    }
    pub fn reset(&self) {
        let _operation = self.operations.lock().unwrap();
        self.authorization.lock().unwrap().invalidate();
        self.engine.lock();
        self.authorization.lock().unwrap().reset(Instant::now());
    }
}
pub fn router(app: App) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(script))
        .route("/app.css", get(style))
        .route("/api/authorize", post(authorize))
        .route("/api/resume", post(resume))
        .route("/api/status", post(status))
        .route("/api/unlock", post(unlock))
        .route("/api/lock", post(lock))
        .route("/api/heartbeat", post(heartbeat))
        .route("/api/command", post(command))
        .layer(DefaultBodyLimit::max(16384))
        .layer(middleware::from_fn_with_state(app.clone(), protect))
        .with_state(app)
}
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error":message}))).into_response()
}
fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error":"Esta janela perdeu a autorização. Abra novamente pelo comando psychotalk.","reauthorize":true}))).into_response()
}
fn validate(app: &App, req: &mut Request) -> Result<Option<AccessGrant>, Box<Response>> {
    let headers = req.headers();
    let origin = format!("http://{}", app.authority);
    if headers.get("host").and_then(|h| h.to_str().ok()) != Some(app.authority.as_str()) {
        return Err(Box::new(error(StatusCode::FORBIDDEN, "Host recusado.")));
    }
    if headers
        .get("origin")
        .is_some_and(|h| h.as_bytes() != origin.as_bytes())
        || headers
            .get("sec-fetch-site")
            .is_some_and(|h| h != "same-origin" && h != "none")
    {
        return Err(Box::new(error(StatusCode::FORBIDDEN, "Origem recusada.")));
    }
    if !req.uri().path().starts_with("/api/") {
        return Ok(None);
    }
    if headers.get("origin").and_then(|h| h.to_str().ok()) != Some(origin.as_str()) {
        return Err(Box::new(error(StatusCode::FORBIDDEN, "Origem exigida.")));
    }
    let tab = headers
        .get("x-psychotalk-tab")
        .and_then(|t| t.to_str().ok())
        .filter(|t| t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| Box::new(error(StatusCode::BAD_REQUEST, "Janela inválida.")))?
        .to_string();
    if matches!(req.uri().path(), "/api/authorize" | "/api/resume") {
        req.extensions_mut().insert(WindowId(tab));
        return Ok(None);
    }
    let supplied = headers
        .get("x-psychotalk-token")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let generation = app
        .authorization
        .lock()
        .unwrap()
        .authenticate(supplied, &tab, Instant::now())
        .ok_or_else(|| Box::new(unauthorized()))?;
    let grant = AccessGrant(generation);
    req.extensions_mut().insert(grant);
    Ok(Some(grant))
}
async fn protect(State(app): State<App>, mut req: Request, next: Next) -> Response {
    let mut response = match validate(&app, &mut req) {
        Ok(grant) => {
            let response = next.run(req).await;
            if grant.is_some_and(|g| !app.authorization.lock().unwrap().current(g.0))
                && response.extensions().get::<CompletedCleanup>().is_none()
            {
                unauthorized()
            } else {
                response
            }
        }
        Err(response) => *response,
    };
    for (name, value) in [
        ("cache-control", "no-store, max-age=0"),
        ("pragma", "no-cache"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-resource-policy", "same-origin"),
        ("cross-origin-opener-policy", "same-origin"),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
    ] {
        response
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    response
}
async fn index() -> Response {
    (
        [("content-type", "text/html; charset=utf-8")],
        include_str!("../web/index.html"),
    )
        .into_response()
}
async fn script() -> Response {
    (
        [("content-type", "text/javascript; charset=utf-8")],
        include_str!("../web/app.js"),
    )
        .into_response()
}
async fn style() -> Response {
    (
        [("content-type", "text/css; charset=utf-8")],
        include_str!("../web/app.css"),
    )
        .into_response()
}
// Validate again while holding the operation gate: requests queued by the old
// document must not execute against a newly authorized/unlocked vault session.
async fn operate(
    app: App,
    grant: AccessGrant,
    work: impl FnOnce(&App) -> Response + Send + 'static,
) -> Response {
    tokio::task::spawn_blocking(move || {
        let _operation = app.operations.lock().unwrap();
        if !app.authorization.lock().unwrap().current(grant.0) {
            return unauthorized();
        }
        work(&app)
    })
    .await
    .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Falha na operação."))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    proof: String,
}
impl Drop for Proof {
    fn drop(&mut self) {
        self.proof.zeroize();
    }
}
async fn exchange(app: App, window: WindowId, input: Proof, kind: ProofKind) -> Response {
    tokio::task::spawn_blocking(move || {
        let _operation = app.operations.lock().unwrap();
        if !app
            .authorization
            .lock()
            .unwrap()
            .consume(kind, &input.proof, Instant::now())
        {
            return unauthorized();
        }
        app.engine.lock();
        let grant = app
            .authorization
            .lock()
            .unwrap()
            .issue(window.0, Instant::now());
        Json(&grant).into_response()
    })
    .await
    .unwrap_or_else(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Falha ao autorizar a janela.",
        )
    })
}
async fn authorize(
    State(app): State<App>,
    Extension(window): Extension<WindowId>,
    Json(input): Json<Proof>,
) -> Response {
    exchange(app, window, input, ProofKind::Bootstrap).await
}
async fn resume(
    State(app): State<App>,
    Extension(window): Extension<WindowId>,
    Json(input): Json<Proof>,
) -> Response {
    exchange(app, window, input, ProofKind::Resume).await
}
async fn status(State(app): State<App>, Extension(grant): Extension<AccessGrant>) -> Response {
    operate(app, grant, |app| Json(app.engine.status()).into_response()).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    password: String,
    #[serde(default)]
    cleanup: Option<String>,
}
impl Drop for Credentials {
    fn drop(&mut self) {
        self.password.zeroize();
        if let Some(p) = &mut self.cleanup {
            p.zeroize();
        }
    }
}
async fn unlock(
    State(app): State<App>,
    Extension(grant): Extension<AccessGrant>,
    Json(input): Json<Credentials>,
) -> Response {
    operate(app, grant, move |app| {
        match app.engine.unlock(
            input.password.as_bytes(),
            input.cleanup.as_deref().map(str::as_bytes),
        ) {
            Ok(v) => {
                let cleaned = v["cleaned"] == true;
                let mut response = Json(v).into_response();
                if cleaned {
                    app.authorization.lock().unwrap().invalidate();
                    response.extensions_mut().insert(CompletedCleanup);
                }
                response
            }
            Err(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
        }
    })
    .await
}
async fn lock(State(app): State<App>, Extension(grant): Extension<AccessGrant>) -> Response {
    {
        let auth = app.authorization.lock().unwrap();
        if !auth.current(grant.0) {
            return unauthorized();
        }
        app.engine.revoke();
    }
    operate(app, grant, |app| {
        app.engine.lock();
        Json(json!({"ok":true})).into_response()
    })
    .await
}
async fn heartbeat(State(app): State<App>, Extension(grant): Extension<AccessGrant>) -> Response {
    operate(app, grant, |app| match app.engine.heartbeat() {
        Ok(()) => Json(json!({"ok":true})).into_response(),
        Err(_) => error(StatusCode::LOCKED, "Sessão bloqueada."),
    })
    .await
}
async fn command(
    State(app): State<App>,
    Extension(grant): Extension<AccessGrant>,
    body: String,
) -> Response {
    let body = Zeroizing::new(body);
    operate(app, grant, move |app| match app.engine.command(&body) {
        Ok(mut v) => {
            let bytes = Zeroizing::new(v.to_string());
            if let Some(messages) = v["view"]["active"]["messages"].as_array_mut() {
                for m in messages {
                    if let Some(Value::String(text)) = m.get_mut("text") {
                        text.zeroize();
                    }
                }
            }
            (
                [("content-type", "application/json")],
                Body::from(bytes.to_string()),
            )
                .into_response()
        }
        Err(_) => error(
            StatusCode::CONFLICT,
            "Operação recusada. Confira a sessão, o código e os participantes.",
        ),
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelopes::{Envelopes, OsStore};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn operation_admitted_before_takeover_cannot_execute_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Arc::new(
            Engine::new(
                Envelopes::new(dir.path(), OsStore::new(dir.path()).unwrap()).unwrap(),
                dir.path().join("unused-tor"),
            )
            .unwrap(),
        );
        let app = App::new(engine, "127.0.0.1:32123".into());
        let (old, resume) = {
            let mut auth = app.authorization.lock().unwrap();
            let bootstrap = auth.bootstrap().to_string();
            assert!(auth.consume(ProofKind::Bootstrap, &bootstrap, Instant::now()));
            let grant = auth.issue("old".into(), Instant::now());
            let old = AccessGrant(
                auth.authenticate(&grant.token, "old", Instant::now())
                    .unwrap(),
            );
            (old, grant.resume.clone())
        };
        let _ = exchange(
            app.clone(),
            WindowId("new".into()),
            Proof { proof: resume },
            ProofKind::Resume,
        )
        .await;
        let ran = Arc::new(AtomicBool::new(false));
        let work_ran = ran.clone();
        let response = operate(app, old, move |_| {
            work_ran.store(true, Ordering::SeqCst);
            Json(json!({"ok":true})).into_response()
        })
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(!ran.load(Ordering::SeqCst));
    }
}
