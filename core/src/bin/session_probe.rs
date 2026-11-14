//! Real Tor + SQLCipher protocol probe. The test driver acknowledges UI snapshots;
//! this does not establish Android rendering or two-device network validation.
use anyhow::{Result, ensure};
use psychotalk_core::{
    session::{Session, Shared},
    vault::Vault,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn snapshot(s: &Arc<Shared>) -> Result<Value> {
    s.heartbeat();
    let mut v = s.command("{\"op\":\"view\"}")?;
    if let Some(cid) = v["view"]["conversations"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|c| c["id"].as_str())
    {
        v = s.command(&json!({"op":"view","conversation":cid}).to_string())?;
        v["view"]["messages"] = v["view"]["active"]["messages"].clone();
        v["view"]["deletion"] = v["view"]["active"]["deletion"].clone();
    }
    s.command(&json!({"op":"displayed","epoch":v["epoch"]}).to_string())?;
    Ok(v)
}
fn pulse(s: &Session) {
    let weak = Arc::downgrade(&s.shared);
    thread::spawn(move || {
        while let Some(s) = weak.upgrade() {
            s.heartbeat();
            drop(s);
            thread::sleep(Duration::from_millis(200));
        }
    });
}
fn wait(
    a: &Session,
    b: Option<&Session>,
    predicate: impl Fn(&Value, Option<&Value>) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut previous = String::new();
    loop {
        let av = snapshot(&a.shared)?;
        let bv = b.map(|s| snapshot(&s.shared)).transpose()?;
        let state = format!(
            "{} / {}",
            av["network"].as_str().unwrap_or("unknown"),
            bv.as_ref()
                .and_then(|v| v["network"].as_str())
                .unwrap_or("offline")
        );
        if state != previous {
            println!("Test session state: {state}");
            previous = state;
        }
        if predicate(&av, bv.as_ref()) {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "session probe stage timed out");
        thread::sleep(Duration::from_millis(100));
    }
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let binary = PathBuf::from(args.next().expect("Tor binary"));
    let root = PathBuf::from(args.next().expect("test directory"));
    std::fs::create_dir_all(root.join("a"))?;
    std::fs::create_dir_all(root.join("b"))?;
    ensure!(
        !root.join("a/vault.db").exists() && !root.join("b/vault.db").exists(),
        "probe requires fresh test vault paths"
    );
    drop(Vault::open(&root.join("a/vault.db"), &[1; 32], true)?);
    drop(Vault::open(&root.join("b/vault.db"), &[2; 32], true)?);
    let a = Session::open(&root.join("a"), &binary, &[1; 32], false)?;
    let b = Session::open(&root.join("b"), &binary, &[2; 32], false)?;
    pulse(&a);
    pulse(&b);
    println!("Real session clients starting (test keys only)");
    wait(&a, Some(&b), |a, b| {
        a["view"]["own_code"].is_string() && b.unwrap()["view"]["own_code"].is_string()
    })?;
    let ac = snapshot(&a.shared)?["view"]["own_code"]
        .as_str()
        .unwrap()
        .to_owned();
    let bc = snapshot(&b.shared)?["view"]["own_code"]
        .as_str()
        .unwrap()
        .to_owned();
    a.shared
        .command(&json!({"op":"pair","code":bc}).to_string())?;
    b.shared
        .command(&json!({"op":"pair","code":ac}).to_string())?;
    a.shared
        .command(&json!({"op":"send","text":"test-a"}).to_string())?;
    b.shared
        .command(&json!({"op":"send","text":"test-b"}).to_string())?;
    wait(&a, Some(&b), |a, b| {
        a["view"]["messages"]
            .as_array()
            .is_some_and(|x| x.len() == 2 && x.iter().all(|m| m["delivered"] == true))
            && b.unwrap()["view"]["messages"]
                .as_array()
                .is_some_and(|x| x.len() == 2 && x.iter().all(|m| m["delivered"] == true))
    })?;
    println!("PASS: bidirectional queued messages persisted and acknowledged over Tor/mTLS");
    b.close();
    a.shared.command("{\"op\":\"delete\"}")?;
    ensure!(
        snapshot(&a.shared)?["view"]["deletion"] == false,
        "offline deletion must remain pending"
    );
    println!("PASS: offline deletion remains pending");
    let b = Session::open(&root.join("b"), &binary, &[2; 32], false)?;
    pulse(&b);
    wait(&a, Some(&b), |a, b| {
        a["view"]["deletion"] == true
            && b.unwrap()["view"]["messages"]
                .as_array()
                .is_some_and(|m| m.len() == 1 && m[0]["mine"] == true)
    })?;
    println!(
        "PASS: reconnection applies author deletion, preserves peer text, acknowledges after commit"
    );
    a.shared
        .command(&json!({"op":"send","text":"new-after-deletion"}).to_string())?;
    wait(&a, Some(&b), |_, b| {
        b.unwrap()["view"]["messages"]
            .as_array()
            .is_some_and(|m| m.len() == 2)
    })?;
    println!("PASS: new messages continue above the persistent deletion watermark");
    a.close();
    b.close();
    Ok(())
}
