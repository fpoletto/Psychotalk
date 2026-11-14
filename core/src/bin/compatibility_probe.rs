//! Frozen 0.1.2 session/storage peer against the current session. Host fixtures only.
pub use psychotalk_core::{contact, tor, transport};
#[allow(dead_code)]
#[path = "../../tests/fixtures/v1/session.rs"]
mod session;
#[allow(dead_code)]
#[path = "../../tests/fixtures/v2/tls.rs"]
pub mod tls;
#[allow(dead_code)]
#[path = "../../tests/fixtures/v1/vault.rs"]
mod vault;
use anyhow::{Result, ensure};
use psychotalk_core::session::Session;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn current(s: &Session) -> Result<Value> {
    let mut v = s.shared.command("{\"op\":\"view\"}")?;
    if let Some(cid) = v["view"]["conversations"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|c| c["id"].as_str())
    {
        v = s
            .shared
            .command(&json!({"op":"view","conversation":cid}).to_string())?;
    }
    s.shared
        .command(&json!({"op":"displayed","epoch":v["epoch"]}).to_string())?;
    Ok(v)
}
fn old(s: &session::Session) -> Result<Value> {
    let v = s.shared.command("{\"op\":\"view\"}")?;
    s.shared
        .command(&json!({"op":"displayed","epoch":v["epoch"]}).to_string())?;
    Ok(v)
}
fn wait(a: &Session, b: &session::Session, p: impl Fn(&Value, &Value) -> bool) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(600);
    while Instant::now() < end {
        if p(&current(a)?, &old(b)?) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("compatibility stage timed out")
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let binary = PathBuf::from(args.next().expect("Tor binary"));
    let root = PathBuf::from(args.next().expect("fresh directory"));
    ensure!(
        !root.join("current/vault.db").exists() && !root.join("legacy/vault.db").exists(),
        "fresh fixtures required"
    );
    let a = Session::open(&root.join("current"), &binary, &[1; 32], true)?;
    let b = session::Session::open(&root.join("legacy"), &binary, &[2; 32], true)?;
    let aw = Arc::downgrade(&a.shared);
    thread::spawn(move || {
        while let Some(s) = aw.upgrade() {
            s.heartbeat();
            drop(s);
            thread::sleep(Duration::from_millis(200));
        }
    });
    let bw = Arc::downgrade(&b.shared);
    thread::spawn(move || {
        while let Some(s) = bw.upgrade() {
            s.heartbeat();
            drop(s);
            thread::sleep(Duration::from_millis(200));
        }
    });
    println!("Current peer and frozen 0.1.2 peer starting over real Tor");
    wait(&a, &b, |a, b| {
        a["view"]["own_code"].is_string() && b["view"]["own_code"].is_string()
    })?;
    let ac = current(&a)?["view"]["own_code"]
        .as_str()
        .unwrap()
        .to_owned();
    let bc = old(&b)?["view"]["own_code"].as_str().unwrap().to_owned();
    a.shared
        .command(&json!({"op":"pair","code":bc}).to_string())?;
    b.shared
        .command(&json!({"op":"pair","code":ac}).to_string())?;
    a.shared
        .command("{\"op\":\"send\",\"text\":\"current fixture\"}")?;
    b.shared
        .command("{\"op\":\"send\",\"text\":\"legacy fixture\"}")?;
    wait(&a, &b, |a, b| {
        a["view"]["active"]["messages"]
            .as_array()
            .is_some_and(|m| m.len() == 2 && m.iter().all(|m| m["delivered"] == true))
            && b["view"]["messages"]
                .as_array()
                .is_some_and(|m| m.len() == 2 && m.iter().all(|m| m["delivered"] == true))
    })?;
    println!(
        "PASS: current and 0.1.2 peer exchange and acknowledge messages over unchanged wire v1"
    );
    a.shared.command("{\"op\":\"delete\"}")?;
    wait(&a, &b, |a, b| {
        a["view"]["active"]["deletion"] == true
            && b["view"]["messages"]
                .as_array()
                .is_some_and(|m| m.len() == 1)
    })?;
    b.shared.command("{\"op\":\"delete\"}")?;
    wait(&a, &b, |a, b| {
        b["view"]["deletion"] == true
            && a["view"]["active"]["messages"]
                .as_array()
                .is_some_and(|m| m.is_empty())
    })?;
    println!("PASS: authenticated deletion interoperates in both directions");
    a.close();
    b.close();
    let migrated =
        psychotalk_core::vault::Vault::open(&root.join("legacy/vault.db"), &[2; 32], false)?;
    ensure!(
        migrated.peers()?.len() == 1,
        "legacy vault migration failed"
    );
    println!("PASS: actual 0.1.2 peer vault migrates after its session closes");
    Ok(())
}
