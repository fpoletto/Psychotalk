//! Three real Tor clients; test driver represents foreground UI snapshots, not Android rendering.
use anyhow::{Result, ensure};
use psychotalk_core::session::{Session, Shared};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
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
fn snapshot(s: &Arc<Shared>, cid: Option<&str>) -> Result<Value> {
    let req = json!({"op":"view","conversation":cid.unwrap_or("")});
    let view = s.command(&req.to_string())?;
    s.command(&json!({"op":"displayed","epoch":view["epoch"]}).to_string())?;
    Ok(view)
}
fn wait(sessions: &[&Session], predicate: impl Fn() -> Result<bool>) -> Result<()> {
    let end = Instant::now() + Duration::from_secs(600);
    while Instant::now() < end {
        for s in sessions {
            snapshot(&s.shared, None)?;
        }
        if predicate()? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    anyhow::bail!("multi-client stage timed out")
}
fn code(s: &Session) -> Result<Option<String>> {
    Ok(snapshot(&s.shared, None)?["view"]["own_code"]
        .as_str()
        .map(str::to_owned))
}
fn pair(s: &Session, code: &str, name: &str) -> Result<String> {
    Ok(s.shared.command(&json!({"op":"pair","code":code,"tagname":name}).to_string())?["view"]["active"]["id"].as_str().unwrap().to_owned())
}
fn send(s: &Session, cid: &str, text: &str) -> Result<()> {
    s.shared
        .command(&json!({"op":"send","conversation":cid,"text":text}).to_string())?;
    Ok(())
}
fn count(s: &Session, cid: &str) -> Result<usize> {
    Ok(
        snapshot(&s.shared, Some(cid))?["view"]["active"]["messages"]
            .as_array()
            .unwrap()
            .len(),
    )
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let binary = PathBuf::from(args.next().expect("Tor binary"));
    let root = PathBuf::from(args.next().expect("fresh fixture directory"));
    for n in ["a", "b", "c"] {
        ensure!(
            !root.join(n).join("vault.db").exists(),
            "fresh fixtures required"
        );
    }
    let a = Session::open(&root.join("a"), &binary, &[1; 32], true)?;
    let b = Session::open(&root.join("b"), &binary, &[2; 32], true)?;
    let c = Session::open(&root.join("c"), &binary, &[3; 32], true)?;
    pulse(&a);
    pulse(&b);
    pulse(&c);
    println!("Starting three independent Tor clients and encrypted vaults");
    wait(&[&a, &b, &c], || {
        Ok(code(&a)?.is_some() && code(&b)?.is_some() && code(&c)?.is_some())
    })?;
    let ab = pair(&a, &code(&b)?.unwrap(), "@bia")?;
    let ac = pair(&a, &code(&c)?.unwrap(), "@caio")?;
    let ba = pair(&b, &code(&a)?.unwrap(), "@ana")?;
    let ca = pair(&c, &code(&a)?.unwrap(), "@ana")?;
    send(&a, &ab, "a to b")?;
    send(&b, &ba, "b to a")?;
    send(&a, &ac, "a to c")?;
    send(&c, &ca, "c to a")?;
    wait(&[&a, &b, &c], || {
        Ok(count(&a, &ab)? == 2
            && count(&a, &ac)? == 2
            && count(&b, &ba)? == 2
            && count(&c, &ca)? == 2)
    })?;
    println!(
        "PASS: three clients, two isolated conversations, bidirectional real onion/mTLS delivery"
    );
    b.close();
    send(&a, &ab, "queued offline")?;
    a.shared
        .command(&json!({"op":"delete","conversation":ab}).to_string())?;
    ensure!(
        snapshot(&a.shared, Some(&ab))?["view"]["active"]["deletion"] == false,
        "offline deletion must remain pending"
    );
    send(&a, &ac, "c remains responsive")?;
    wait(&[&a, &c], || Ok(count(&c, &ca)? == 3))?;
    println!("PASS: an offline peer and pending deletion do not block the other conversation");
    let b = Session::open(&root.join("b"), &binary, &[2; 32], false)?;
    pulse(&b);
    wait(&[&a, &b, &c], || {
        Ok(
            snapshot(&a.shared, Some(&ab))?["view"]["active"]["deletion"] == true
                && count(&b, &ba)? == 1,
        )
    })?;
    ensure!(count(&c, &ca)? == 3, "cross-conversation deletion");
    send(&a, &ab, "after deletion")?;
    wait(&[&a, &b, &c], || Ok(count(&b, &ba)? == 2))?;
    a.shared
        .command(&json!({"op":"rename","conversation":ab,"tagname":"@melhor_amiga"}).to_string())?;
    ensure!(
        snapshot(&a.shared, Some(&ab))?["view"]["active"]["tagname"] == "@melhor_amiga",
        "rename failed"
    );
    println!(
        "PASS: deletion applied after reconnect; unrelated history preserved; new messages and local tagname work"
    );
    a.close();
    b.close();
    c.close();
    Ok(())
}
