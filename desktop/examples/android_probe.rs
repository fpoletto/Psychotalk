//! Disposable real OS-keyring vault talking to the installed Android APK through public Tor.
use anyhow::{Result, ensure};
use psychotalk_core::session::Session;
use psychotalk_desktop::envelopes::{Envelopes, OsStore, Unlock, private_dir};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn view(s: &Session, cid: &str) -> Result<Value> {
    let v = s
        .shared
        .command(&json!({"op":"view","conversation":cid}).to_string())?;
    s.shared
        .command(&json!({"op":"displayed","epoch":v["epoch"]}).to_string())?;
    Ok(v)
}
fn wait(s: &Session, cid: &str, test: impl Fn(&Value) -> bool) -> Result<Value> {
    let end = Instant::now() + Duration::from_secs(900);
    let mut report = Instant::now();
    while Instant::now() < end {
        let v = view(s, cid)?;
        if report.elapsed() > Duration::from_secs(20) {
            println!(
                "Fixture status: network={}, connections={}, messages={}, delivered={}",
                v["network"],
                v["connections"]
                    .as_object()
                    .map_or(0, |c| c.values().filter(|v| **v == "Conectado").count()),
                v["view"]["active"]["messages"]
                    .as_array()
                    .map_or(0, Vec::len),
                v["view"]["active"]["messages"]
                    .as_array()
                    .map_or(0, |m| m.iter().filter(|m| m["delivered"] == true).count())
            );
            report = Instant::now();
        }
        if test(&v) {
            return Ok(v);
        }
        thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("desktop/Android Tor stage timed out")
}
fn open(e: &Envelopes<OsStore>, tor: &std::path::Path) -> Result<Session> {
    let Unlock::Normal(k) = e.unlock(b"desktop interop normal")? else {
        anyhow::bail!("fixture unlock failed")
    };
    let s = Session::open(&e.root, tor, &k, false)?;
    let w = Arc::downgrade(&s.shared);
    thread::spawn(move || {
        while let Some(s) = w.upgrade() {
            s.heartbeat();
            drop(s);
            thread::sleep(Duration::from_millis(200));
        }
    });
    Ok(s)
}
fn main() -> Result<()> {
    let mut a = std::env::args().skip(1);
    let tor = PathBuf::from(a.next().expect("Tor executable"));
    let base = PathBuf::from(a.next().expect("fresh disposable fixture"));
    ensure!(!base.join("vault").exists(), "fresh fixture required");
    private_dir(&base)?;
    let e = Envelopes::new(&base, OsStore::new(&base)?)?;
    e.setup(b"desktop interop normal", b"desktop interop cleanup")?;
    if let Some(cache) = a.next().filter(|cache| cache != "-") {
        std::fs::create_dir_all(e.root.join("tor"))?;
        for file in std::fs::read_dir(cache)? {
            let file = file?;
            if file.file_name().to_string_lossy().starts_with("cached-")
                && file.file_type()?.is_file()
            {
                std::fs::copy(file.path(), e.root.join("tor").join(file.file_name()))?;
            }
        }
    }
    let s = open(&e, &tor)?;
    let code = wait(&s, "", |v| v["view"]["own_code"].is_string())?["view"]["own_code"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::write(base.join("desktop.code"), &code)?;
    println!("READY: real desktop SQLCipher + OS keyring; waiting Android public code");
    wait(&s, "", |_| base.join("android.code").is_file())?;
    let android = fs::read_to_string(base.join("android.code"))?;
    let paired = s
        .shared
        .command(&json!({"op":"pair","code":android}).to_string())?;
    let cid = paired["view"]["active"]["id"].as_str().unwrap();
    s.shared.command(
        &json!({"op":"send","conversation":cid,"text":"desktop to Android"}).to_string(),
    )?;
    wait(&s, cid, |v| {
        v["view"]["active"]["messages"]
            .as_array()
            .is_some_and(|m| m.len() == 2 && m.iter().all(|m| m["delivered"] == true))
    })?;
    fs::write(base.join("desktop.exchange"), b"ok")?;
    println!("PASS: desktop ↔ Android actual Tor/mTLS, bidirectional persisted messages");
    wait(&s, cid, |v| {
        v["view"]["active"]["messages"]
            .as_array()
            .is_some_and(|m| m.len() == 1 && m[0]["text"] == "desktop to Android")
    })?;
    println!("PASS: Android author deletion preserved desktop author's message");
    s.shared
        .command(&json!({"op":"delete","conversation":cid}).to_string())?;
    wait(&s, cid, |v| {
        v["view"]["active"]["deletion"] == true
            && v["view"]["active"]["messages"]
                .as_array()
                .is_some_and(|m| m.is_empty())
    })?;
    fs::write(base.join("desktop.confirmed"), b"ok")?;
    println!("PASS: desktop author deletion acknowledged by Android after UI epoch");
    if a.next().as_deref() == Some("--groups") {
        group_probe(&base, &s, &tor, cid)?;
    }
    s.close();
    let s = open(&e, &tor)?;
    let restored = wait(&s, "", |v| v["view"]["own_code"].is_string())?;
    ensure!(
        restored["view"]["own_code"] == code,
        "desktop identity changed"
    );
    s.close();
    e.wipe()?;
    println!("PASS: real OS-keyring reopen preserves identity, cleanup invalidates credential");
    Ok(())
}

fn pair_wait(
    a: &Session,
    b: &Session,
    gid: &str,
    predicate: impl Fn(&Value, &Value) -> bool,
) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(900);
    let mut report = Instant::now() - Duration::from_secs(21);
    while Instant::now() < end {
        let av = view(a, gid)?;
        let bv = view(b, gid)?;
        if report.elapsed() > Duration::from_secs(20) {
            println!(
                "Mixed status: A network={} joined={} ready={} messages={}; B network={} joined={} ready={} messages={}",
                av["network"],
                av["view"]["active"]["group"]["joined"],
                av["view"]["active"]["group"]["ready"],
                av["view"]["active"]["messages"]
                    .as_array()
                    .map_or(0, Vec::len),
                bv["network"],
                bv["view"]["active"]["group"]["joined"],
                bv["view"]["active"]["group"]["ready"],
                bv["view"]["active"]["messages"]
                    .as_array()
                    .map_or(0, Vec::len)
            );
            report = Instant::now();
        }
        if predicate(&av["view"]["active"], &bv["view"]["active"]) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("mixed group stage timed out")
}
fn group_probe(
    base: &std::path::Path,
    a: &Session,
    tor: &std::path::Path,
    android_cid: &str,
) -> Result<()> {
    let other = base.join("groupmate");
    private_dir(&other)?;
    let e = Envelopes::new(&other, OsStore::new(&other)?)?;
    e.setup(b"desktop interop normal", b"desktop interop cleanup")?;
    std::fs::create_dir_all(e.root.join("tor"))?;
    for file in std::fs::read_dir(base.join("vault/tor"))? {
        let file = file?;
        if file.file_name().to_string_lossy().starts_with("cached-") && file.file_type()?.is_file()
        {
            std::fs::copy(file.path(), e.root.join("tor").join(file.file_name()))?;
        }
    }
    let b = open(&e, tor)?;
    let code = wait(&b, "", |v| v["view"]["own_code"].is_string())?["view"]["own_code"]
        .as_str()
        .unwrap()
        .to_owned();
    let peer = a
        .shared
        .command(&json!({"op":"pair","code":code}).to_string())?["view"]["active"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let created=a.shared.command(&json!({"op":"create_group","tagname":"Mixed desktop Android","members":[android_cid,peer]}).to_string())?;
    let gid = created["view"]["active"]["id"].as_str().unwrap();
    let invitation = created["view"]["active"]["group"]["invitation"]
        .as_str()
        .unwrap();
    b.shared
        .command(&json!({"op":"import_group","code":invitation}).to_string())?;
    fs::write(base.join("group.invite"), invitation)?;
    pair_wait(a, &b, gid, |a, b| {
        a["group"]["ready"] == 3 && b["group"]["ready"] == 3
    })?;
    a.shared.command(
        &json!({"op":"send","conversation":gid,"text":"desktop A group author"}).to_string(),
    )?;
    b.shared.command(
        &json!({"op":"send","conversation":gid,"text":"desktop B group author"}).to_string(),
    )?;
    pair_wait(a, &b, gid, |a, b| {
        [a, b].iter().all(|v| {
            v["messages"].as_array().is_some_and(|m| {
                m.len() == 3
                    && m.iter()
                        .any(|m| m["mine"] == true && m["delivered_count"] == 2)
            })
        })
    })?;
    fs::write(base.join("group.exchange"), b"ok")?;
    println!(
        "PASS: mixed group desktop A + desktop B + Android, all authors delivered to every recipient"
    );
    pair_wait(a, &b, gid, |a, b| {
        [a, b].iter().all(|v| {
            v["messages"].as_array().is_some_and(|m| {
                m.len() == 2 && m.iter().all(|m| m["text"] != "Android group author")
            })
        })
    })?;
    a.shared
        .command(&json!({"op":"delete","conversation":gid}).to_string())?;
    pair_wait(a, &b, gid, |a, b| {
        a["deletion"] == true
            && [a, b].iter().all(|v| {
                v["messages"]
                    .as_array()
                    .is_some_and(|m| m.len() == 1 && m[0]["text"] == "desktop B group author")
            })
    })?;
    b.shared
        .command(&json!({"op":"delete","conversation":gid}).to_string())?;
    pair_wait(a, &b, gid, |a, b| {
        b["deletion"] == true && a["messages"].as_array().is_some_and(|m| m.is_empty())
    })?;
    fs::write(base.join("group.confirmed"), b"ok")?;
    wait(a, gid, |_| base.join("android.offline").exists())?;
    a.shared.command(
        &json!({"op":"send","conversation":gid,"text":"covered offline text"}).to_string(),
    )?;
    a.shared
        .command(&json!({"op":"delete","conversation":gid}).to_string())?;
    b.shared.command(
        &json!({"op":"send","conversation":gid,"text":"other author during offline"}).to_string(),
    )?;
    pair_wait(a, &b, gid, |a, b| {
        a["group"]["deletion"]["confirmed"] == 1
            && a["deletion"] == false
            && b["messages"].as_array().is_some_and(|m| m.len() == 1)
    })?;
    fs::write(base.join("group.reopen"), b"ok")?;
    println!("PASS: offline Android keeps its recipient ACK pending; other group author continues");
    pair_wait(a, &b, gid, |a, b| {
        a["deletion"] == true
            && b["messages"]
                .as_array()
                .is_some_and(|m| m.len() == 1 && m[0]["delivered_count"] == 2)
    })?;
    fs::write(base.join("group.final.confirmed"), b"ok")?;
    println!(
        "PASS: reopened Android applies author-only deletion and every recipient confirms; other author preserved"
    );
    b.close();
    e.wipe()?;
    Ok(())
}
