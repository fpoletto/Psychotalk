//! Real-network acceptance probe; temporary identities, no production vault.
use anyhow::{Result, ensure};
use psychotalk_core::{
    tls::{Identity, configs},
    tor::Tor,
    transport::{onion_stream, read_frame, write_frame},
};
use rustls::{ClientConnection, ServerConnection, StreamOwned, pki_types::ServerName};
use std::{
    net::TcpListener,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn receive(
    listener: TcpListener,
    config: Arc<rustls::ServerConfig>,
) -> thread::JoinHandle<Result<()>> {
    thread::spawn(move || {
        let (socket, _) = listener.accept()?;
        socket.set_read_timeout(Some(Duration::from_secs(30)))?;
        socket.set_write_timeout(Some(Duration::from_secs(30)))?;
        let mut stream = StreamOwned::new(ServerConnection::new(config)?, socket);
        ensure!(
            read_frame(&mut stream)? == b"probe-one",
            "unexpected request"
        );
        write_frame(&mut stream, b"probe-two")?;
        Ok(())
    })
}
fn run(binary: &Path, root: PathBuf) -> Result<()> {
    let a = Identity::generate()?;
    let b = Identity::generate()?;
    let (ac, asrv) = configs(&a, b.pin())?;
    let (bc, bsrv) = configs(&b, a.pin())?;
    let al = TcpListener::bind("127.0.0.1:0")?;
    let bl = TcpListener::bind("127.0.0.1:0")?;
    let mut at = Tor::start(binary, &root.join("a"), al.local_addr()?.port(), None)?;
    let mut bt = Tor::start(binary, &root.join("b"), bl.local_addr()?.port(), None)?;
    at.enable_network()?;
    bt.enable_network()?;
    println!("Two independent Tor clients started; waiting for bootstrap");
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut previous = (255, 255);
    loop {
        let progress = (at.bootstrap_progress()?, bt.bootstrap_progress()?);
        if progress != previous {
            println!("Bootstrap: {}% / {}%", progress.0, progress.1);
            previous = progress;
        }
        if progress == (100, 100) {
            break;
        }
        ensure!(Instant::now() < deadline, "bootstrap timed out");
        thread::sleep(Duration::from_secs(2));
    }
    println!("Both Tor clients bootstrapped; publishing onion services");
    thread::sleep(Duration::from_secs(20));
    let servers = [receive(bl, bsrv), receive(al, asrv)];
    for (client, proxy, host) in [
        (ac, at.proxy, bt.onion.clone()),
        (bc, bt.proxy, at.onion.clone()),
    ] {
        let mut last = None;
        for _ in 0..6 {
            match onion_stream(proxy, &host) {
                Ok(socket) => {
                    last = Some(socket);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_secs(5)),
            }
        }
        let socket = last.ok_or_else(|| anyhow::anyhow!("onion connection failed"))?;
        let mut stream = StreamOwned::new(
            ClientConnection::new(client, ServerName::try_from("psychotalk.invalid")?)?,
            socket,
        );
        write_frame(&mut stream, b"probe-one")?;
        ensure!(
            read_frame(&mut stream)? == b"probe-two",
            "unexpected response"
        );
    }
    for server in servers {
        server
            .join()
            .map_err(|_| anyhow::anyhow!("receiver failed"))??;
    }
    println!("PASS: bidirectional onion communication with pinned mutual TLS 1.3");
    let proxy = at.proxy;
    at.stop();
    ensure!(
        onion_stream(proxy, &bt.onion).is_err(),
        "fail-closed violation"
    );
    println!("PASS: stopped Tor refuses new sends; no alternate transport");
    Ok(())
}
fn main() -> Result<()> {
    let binary = std::env::args()
        .nth(1)
        .expect("usage: onion_probe TOR_BINARY TEST_DIRECTORY");
    let root = std::env::args().nth(2).expect("test directory required");
    run(Path::new(&binary), PathBuf::from(root))
}
