use crate::{
    tls,
    tor::Tor,
    transport,
    vault::{Packet, Vault},
};
use anyhow::{Result, ensure};
use rustls::{ClientConnection, ServerConnection, StreamOwned, pki_types::ServerName};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::Path,
    process::Child,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    op: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    epoch: u64,
}
impl Drop for Request {
    fn drop(&mut self) {
        self.text.zeroize();
        self.code.zeroize();
    }
}

pub struct Shared {
    vault: Mutex<Vault>,
    running: AtomicBool,
    lease: Mutex<Instant>,
    sockets: Mutex<Vec<TcpStream>>,
    killer: Mutex<Option<Arc<Mutex<Child>>>>,
    status: Mutex<&'static str>,
    redaction: AtomicU64,
    displayed: AtomicU64,
}
pub struct Session {
    pub shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
}
impl Shared {
    fn alive(&self) -> bool {
        if self.lease.lock().unwrap().elapsed() > Duration::from_secs(2) {
            self.revoke();
        }
        self.running.load(Ordering::SeqCst)
    }
    pub fn heartbeat(&self) {
        if self.running.load(Ordering::SeqCst) {
            let expired = {
                let mut lease = self.lease.lock().unwrap();
                let expired = lease.elapsed() > Duration::from_secs(2);
                if !expired {
                    *lease = Instant::now();
                }
                expired
            };
            if expired {
                self.revoke();
            }
        }
    }
    pub fn revoke(&self) {
        self.running.store(false, Ordering::SeqCst);
        for s in self.sockets.lock().unwrap().iter() {
            let _ = s.shutdown(Shutdown::Both);
        }
        if let Some(child) = self.killer.lock().unwrap().as_ref() {
            let _ = child.lock().unwrap().kill();
        }
    }
    pub fn command(&self, input: &str) -> Result<Value> {
        ensure!(self.alive(), "locked");
        let req: Request = serde_json::from_str(input)?;
        if req.op == "displayed" {
            self.displayed.fetch_max(
                req.epoch.min(self.redaction.load(Ordering::SeqCst)),
                Ordering::SeqCst,
            );
            return Ok(json!({"ok":true}));
        }
        let mut vault = self.vault.lock().unwrap();
        ensure!(self.alive(), "locked");
        match req.op.as_str() {
            "pair" => {
                vault.pair(&req.code)?;
            }
            "send" => {
                vault.send(&req.text)?;
            }
            "delete" => {
                vault.delete_all()?;
            }
            "view" => {}
            _ => anyhow::bail!("unknown operation"),
        }
        Ok(
            json!({"view":vault.view()?,"network":*self.status.lock().unwrap(),"epoch":self.redaction.load(Ordering::SeqCst)}),
        )
    }
    fn status(&self, text: &'static str) {
        *self.status.lock().unwrap() = text;
    }
    fn register(&self, s: &TcpStream) -> Result<()> {
        let mut sockets = self.sockets.lock().unwrap();
        sockets.clear();
        sockets.push(s.try_clone()?);
        Ok(())
    }
    fn apply(&self, p: Packet) -> Result<Option<Packet>> {
        let deletion = matches!(p.content, crate::vault::Content::Delete { .. });
        let reply = {
            let mut v = self.vault.lock().unwrap();
            ensure!(self.alive(), "locked");
            v.apply(p)?
        };
        if deletion {
            // ACK also waits for the official UI to discard its visible snapshot.
            let epoch = self.redaction.fetch_add(1, Ordering::SeqCst) + 1;
            while self.displayed.load(Ordering::SeqCst) < epoch {
                pause(self, 50)?;
            }
        }
        Ok(reply)
    }
}
impl Session {
    pub fn open(directory: &Path, tor_binary: &Path, key: &[u8], create: bool) -> Result<Self> {
        ensure!(
            !directory.join("wipe.intent").exists() && !directory.join("setup.intent").exists(),
            "cleanup required"
        );
        std::fs::create_dir_all(directory)?;
        let vault = Vault::open(&directory.join("vault.db"), key, create)?;
        let shared = Arc::new(Shared {
            vault: Mutex::new(vault),
            running: AtomicBool::new(true),
            lease: Mutex::new(Instant::now()),
            sockets: Mutex::new(Vec::new()),
            killer: Mutex::new(None),
            status: Mutex::new("Conectando ao Tor"),
            redaction: AtomicU64::new(0),
            displayed: AtomicU64::new(0),
        });
        let s = shared.clone();
        let directory = directory.to_owned();
        let binary = tor_binary.to_owned();
        let worker = thread::spawn(move || {
            while s.alive() {
                let result = network(&s, &directory, &binary);
                #[cfg(all(feature = "probe-diagnostics", not(target_os = "android")))]
                if let Err(ref e) = result {
                    eprintln!("Test-fixture network stage: {e:?}");
                }
                let _ = result;
                s.killer.lock().unwrap().take();
                s.status("Tor indisponível — mensagens pendentes");
                if pause(&s, 2000).is_err() {
                    break;
                }
            }
            s.status("Rede encerrada — desbloqueie novamente");
        });
        let guard = shared.clone();
        let watchdog = thread::spawn(move || {
            while guard.alive() {
                thread::sleep(Duration::from_millis(100));
            }
        });
        Ok(Self {
            shared,
            worker: Some(worker),
            watchdog: Some(watchdog),
        })
    }
    pub fn close(mut self) {
        self.shared.revoke();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if let Some(w) = self.watchdog.take() {
            let _ = w.join();
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.shared.revoke();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if let Some(w) = self.watchdog.take() {
            let _ = w.join();
        }
    }
}
fn pause(s: &Shared, millis: u64) -> Result<()> {
    for _ in 0..millis.div_ceil(100) {
        ensure!(s.alive(), "locked");
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
fn network(s: &Shared, dir: &Path, binary: &Path) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let key = s
        .vault
        .lock()
        .unwrap()
        .get("onion-key")?
        .map(Zeroizing::new);
    let keystr = key.as_ref().map(|b| std::str::from_utf8(b)).transpose()?;
    let mut tor = Tor::start(
        binary,
        &dir.join("tor"),
        listener.local_addr()?.port(),
        keystr,
    )?;
    *s.killer.lock().unwrap() = Some(tor.child_handle());
    {
        let v = s.vault.lock().unwrap();
        ensure!(s.alive(), "locked");
        v.put("onion-key", tor.onion_key.as_bytes())?;
        v.put("onion", tor.onion.as_bytes())?;
    }
    ensure!(s.alive(), "locked");
    tor.enable_network()?;
    while s.alive() && !tor.bootstrapped()? {
        pause(s, 500)?;
    }
    while s.alive() {
        let (contact, pin) = {
            let v = s.vault.lock().unwrap();
            (v.contact()?, v.identity.pin())
        };
        let Some(contact) = contact else {
            s.status("Tor pronto — aguardando pareamento");
            pause(s, 500)?;
            continue;
        };
        let (client, server) = {
            let v = s.vault.lock().unwrap();
            tls::configs(&v.identity, contact.pin)?
        };
        s.status("Tor pronto — contato offline");
        let result = if pin < contact.pin {
            transport::onion_stream(tor.proxy, &contact.onion).and_then(|socket| {
                s.register(&socket)?;
                socket.set_nonblocking(false)?;
                socket.set_read_timeout(Some(Duration::from_secs(30)))?;
                socket.set_write_timeout(Some(Duration::from_secs(30)))?;
                let mut tls = StreamOwned::new(
                    ClientConnection::new(client, ServerName::try_from("psychotalk.invalid")?)?,
                    socket,
                );
                while tls.conn.is_handshaking() {
                    ensure!(s.alive(), "locked");
                    tls.conn.complete_io(&mut tls.sock)?;
                }
                let mut ack = None;
                while s.alive() {
                    let mut batch = Vec::new();
                    if let Some(p) = ack.take() {
                        batch.push(p);
                    }
                    write_batch(&mut tls, &batch, s)?;
                    for p in read_batch(&mut tls)? {
                        if let Some(reply) = s.apply(p)? {
                            ensure!(ack.is_none(), "too many requests");
                            ack = Some(reply);
                        }
                    }
                    s.status("Conectado ao contato por Tor");
                    pause(s, 500)?;
                }
                Ok(())
            })
        } else {
            match listener.accept() {
                Ok((socket, _)) => {
                    s.register(&socket)?;
                    // accept() can inherit O_NONBLOCK on Darwin; framed TLS uses bounded blocking I/O.
                    socket.set_nonblocking(false)?;
                    socket.set_read_timeout(Some(Duration::from_secs(30)))?;
                    socket.set_write_timeout(Some(Duration::from_secs(30)))?;
                    let mut tls = StreamOwned::new(ServerConnection::new(server)?, socket);
                    (|| -> Result<()> {
                        while tls.conn.is_handshaking() {
                            ensure!(s.alive(), "locked");
                            tls.conn.complete_io(&mut tls.sock)?;
                        }
                        while s.alive() {
                            let mut batch = Vec::new();
                            for p in read_batch(&mut tls)? {
                                if let Some(ack) = s.apply(p)? {
                                    ensure!(batch.is_empty(), "too many requests");
                                    batch.push(ack);
                                }
                            }
                            write_batch(&mut tls, &batch, s)?;
                            s.status("Conectado ao contato por Tor");
                        }
                        Ok(())
                    })()
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    pause(s, 500)?;
                    continue;
                }
                Err(_) => anyhow::bail!("listener failed"),
            }
        };
        s.sockets.lock().unwrap().clear();
        #[cfg(all(feature = "probe-diagnostics", not(target_os = "android")))]
        if let Err(ref e) = result {
            eprintln!("Test-fixture connection stage: {e:?}");
        }
        if result.is_err() {
            s.status("Tor pronto — contato offline");
            if pin < contact.pin {
                let _ = tor.refresh_service(&contact.onion);
            }
            pause(s, 2000)?;
        }
        if !tor.bootstrapped()? {
            s.status("Tor indisponível — mensagens pendentes");
            pause(s, 2000)?;
        }
    }
    Ok(())
}
fn write_batch(stream: &mut impl Write, batch: &[Packet], s: &Shared) -> Result<()> {
    // Serialize selection and transmission with deletion, so a canceled body cannot be selected earlier and sent later.
    let v = s.vault.lock().unwrap();
    ensure!(s.alive(), "locked");
    let mut batch = batch.to_vec();
    batch.push(v.next()?);
    let bytes = Zeroizing::new(serde_json::to_vec(&batch)?);
    transport::write_frame(stream, &bytes)
}
fn read_batch(stream: &mut impl Read) -> Result<Vec<Packet>> {
    let bytes = Zeroizing::new(transport::read_frame(stream)?);
    let batch: Vec<Packet> = serde_json::from_slice(&bytes)?;
    ensure!(!batch.is_empty() && batch.len() <= 2, "invalid batch");
    ensure!(
        batch
            .iter()
            .filter(|p| matches!(
                p.content,
                crate::vault::Content::Message { .. }
                    | crate::vault::Content::Delete { .. }
                    | crate::vault::Content::Poll
            ))
            .count()
            == 1,
        "invalid request count"
    );
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Shared) {
        let root = tempfile::TempDir::new().unwrap();
        let vault = Vault::open(&root.path().join("vault"), &[9; 32], true).unwrap();
        (
            root,
            Shared {
                vault: Mutex::new(vault),
                running: AtomicBool::new(true),
                lease: Mutex::new(Instant::now()),
                sockets: Mutex::new(Vec::new()),
                killer: Mutex::new(None),
                status: Mutex::new("offline test fixture"),
                redaction: AtomicU64::new(0),
                displayed: AtomicU64::new(0),
            },
        )
    }
    #[test]
    fn expired_heartbeat_cannot_resurrect_session() {
        let (_root, s) = fixture();
        *s.lease.lock().unwrap() = Instant::now() - Duration::from_secs(3);
        s.heartbeat();
        assert!(!s.running.load(Ordering::SeqCst));
        assert!(s.command("{\"op\":\"view\"}").is_err());
    }
    #[test]
    fn android_command_boundary_queues_and_deletes_offline() {
        use sha3::{Digest, Sha3_256};
        let (_root, s) = fixture();
        let key = [7; 32];
        let mut hash = Sha3_256::new();
        hash.update(b".onion checksum");
        hash.update(key);
        hash.update([3]);
        let mut raw = key.to_vec();
        raw.extend_from_slice(&hash.finalize()[..2]);
        raw.push(3);
        let contact = crate::contact::Contact {
            onion: format!(
                "{}.onion",
                data_encoding::BASE32_NOPAD
                    .encode(&raw)
                    .to_ascii_lowercase()
            ),
            pin: [42; 32],
        };
        s.command(&json!({"op":"pair","code":contact.code()}).to_string())
            .unwrap();
        let view = s
            .command("{\"op\":\"send\",\"text\":\"offline test\"}")
            .unwrap();
        assert_eq!(view["view"]["messages"][0]["delivered"], false);
        let view = s.command("{\"op\":\"delete\"}").unwrap();
        assert!(view["view"]["messages"].as_array().unwrap().is_empty());
        assert_eq!(view["view"]["deletion"], false);
        assert!(
            s.command("{\"op\":\"send\",\"text\":\"x\",\"unexpected\":true}")
                .is_err()
        );
    }
}
