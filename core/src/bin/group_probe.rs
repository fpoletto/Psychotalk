//! Real public Tor integration using disposable host vaults and UI epoch acknowledgements.
use anyhow::{Result, ensure};
use psychotalk_core::session::Session;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
fn pulse(s: &Session) {
    let w = Arc::downgrade(&s.shared);
    thread::spawn(move || {
        while let Some(s) = w.upgrade() {
            s.heartbeat();
            drop(s);
            thread::sleep(Duration::from_millis(200));
        }
    });
}
fn view(s: &Session, cid: &str) -> Result<Value> {
    let v = s
        .shared
        .command(&json!({"op":"view","conversation":cid}).to_string())?;
    s.shared
        .command(&json!({"op":"displayed","epoch":v["epoch"]}).to_string())?;
    Ok(v)
}
fn wait(nodes: &[Option<Session>], predicate: impl Fn() -> Result<bool>) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(600);
    let mut report = Instant::now();
    while Instant::now() < end {
        if report.elapsed() > Duration::from_secs(30) {
            for (i, s) in nodes.iter().enumerate() {
                if let Some(s) = s {
                    let v = view(s, "")?;
                    println!(
                        "Fixture {i}: network={}, connections={}",
                        v["network"], v["connections"]
                    );
                }
            }
            report = Instant::now();
        }
        for s in nodes.iter().flatten() {
            view(s, "")?;
        }
        if predicate()? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("group Tor stage timed out")
}
fn active(s: &Session, gid: &str) -> Result<Value> {
    Ok(view(s, gid)?["view"]["active"].clone())
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let binary = PathBuf::from(args.next().expect("Tor executable"));
    let root = PathBuf::from(args.next().expect("fresh fixture root"));
    let n: usize = args.next().expect("3 or 5 participants").parse()?;
    ensure!(n == 3 || n == 5, "3 or 5 participants");
    let mut nodes = Vec::new();
    for i in 0..n {
        let dir = root.join(i.to_string());
        ensure!(!dir.join("vault.db").exists(), "fresh fixture required");
        let s = Session::open(&dir, &binary, &[i as u8 + 1; 32], true)?;
        pulse(&s);
        nodes.push(Some(s));
    }
    println!("Starting {n} independent public Tor clients");
    wait(&nodes, || {
        for s in nodes.iter().flatten() {
            if !view(s, "")?["view"]["own_code"].is_string() {
                return Ok(false);
            }
        }
        Ok(true)
    })?;
    let codes: Vec<_> = nodes
        .iter()
        .flatten()
        .map(|s| {
            Ok(view(s, "")?["view"]["own_code"]
                .as_str()
                .unwrap()
                .to_owned())
        })
        .collect::<Result<_>>()?;
    let a = nodes[0].as_ref().unwrap();
    let mut cids = Vec::new();
    for code in &codes[1..] {
        let v = a
            .shared
            .command(&json!({"op":"pair","code":code}).to_string())?;
        cids.push(v["view"]["active"]["id"].as_str().unwrap().to_owned());
    }
    let group = a.shared.command(
        &json!({"op":"create_group","tagname":"Real Tor group","members":cids}).to_string(),
    )?;
    let gid = group["view"]["active"]["id"].as_str().unwrap().to_owned();
    let invite = group["view"]["active"]["group"]["invitation"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = nodes[1].as_ref().unwrap();
    b.shared
        .command(&json!({"op":"pair","code":codes[0]}).to_string())?;
    a.shared.command(
        &json!({"op":"send","conversation":cids[0],"text":"DM while group awaiting consent"})
            .to_string(),
    )?;
    wait(&nodes, || {
        Ok(active(b, &cids[0])?["messages"].as_array().unwrap().len() == 1)
    })?;
    println!(
        "PASS: unaccepted group does not block existing DM and does not authorize group traffic"
    );
    for s in nodes.iter().flatten().skip(1) {
        let preview = s
            .shared
            .command(&json!({"op":"preview_group","code":invite}).to_string())?;
        ensure!(preview["preview"]["id"] == gid, "preview mismatch");
        s.shared
            .command(&json!({"op":"import_group","code":invite}).to_string())?;
    }
    wait(&nodes, || {
        for s in nodes.iter().flatten() {
            if active(s, &gid)?["group"]["ready"] != n {
                return Ok(false);
            }
        }
        Ok(true)
    })?;
    for (i, s) in nodes.iter().enumerate() {
        s.as_ref().unwrap().shared.command(
            &json!({"op":"send","conversation":gid,"text":format!("author {i}")}).to_string(),
        )?;
    }
    wait(&nodes, || {
        for s in nodes.iter().flatten() {
            let c = active(s, &gid)?;
            let m = c["messages"].as_array().unwrap();
            if m.len() != n
                || !m
                    .iter()
                    .any(|m| m["mine"] == true && m["delivered_count"] == n - 1)
            {
                return Ok(false);
            }
        }
        Ok(true)
    })?;
    println!(
        "PASS: fixed {n}-participant group, all authors delivered directly over onion/mTLS with {receipts} recipient receipts",
        receipts = n - 1
    );
    nodes[n - 1].take().unwrap().close();
    let a = nodes[0].as_ref().unwrap();
    a.shared
        .command(&json!({"op":"delete","conversation":gid}).to_string())?;
    wait(&nodes, || {
        Ok(active(a, &gid)?["group"]["deletion"]["confirmed"] == n - 2)
    })?;
    ensure!(
        active(a, &gid)?["deletion"] == false,
        "offline recipient must remain pending"
    );
    let b = nodes[1].as_ref().unwrap();
    b.shared.command(
        &json!({"op":"send","conversation":gid,"text":"other author survives offline peer"})
            .to_string(),
    )?;
    wait(&nodes, || {
        Ok(active(a, &gid)?["messages"].as_array().unwrap().len() == n)
    })?;
    println!("PASS: offline recipient remains pending; other authors and DM continue");
    let s = Session::open(
        &root.join((n - 1).to_string()),
        &binary,
        &[n as u8; 32],
        false,
    )?;
    pulse(&s);
    nodes[n - 1] = Some(s);
    wait(&nodes, || {
        if active(nodes[0].as_ref().unwrap(), &gid)?["deletion"] != true {
            return Ok(false);
        }
        for s in nodes.iter().flatten() {
            let c = active(s, &gid)?;
            let m = c["messages"].as_array().unwrap();
            if m.len() != n || m.iter().any(|m| m["text"] == "author 0") {
                return Ok(false);
            }
        }
        Ok(true)
    })?;
    let a = nodes[0].as_ref().unwrap();
    a.shared.command(
        &json!({"op":"send","conversation":gid,"text":"new sequence after deletion"}).to_string(),
    )?;
    wait(&nodes, || {
        for s in nodes.iter().flatten() {
            if active(s, &gid)?["messages"].as_array().unwrap().len() != n + 1 {
                return Ok(false);
            }
        }
        Ok(true)
    })?;
    ensure!(
        active(nodes[1].as_ref().unwrap(), &cids[0])?["messages"]
            .as_array()
            .unwrap()
            .len()
            == 1,
        "group deletion changed DM"
    );
    println!(
        "PASS: reopened recipient applied author-only deletion, every recipient confirmed; later messages and DM preserved"
    );
    for s in nodes.into_iter().flatten() {
        s.close();
    }
    Ok(())
}
