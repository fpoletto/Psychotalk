use crate::{
    groups::GroupEvent,
    tls,
    tor::Tor,
    transport,
    vault::{Content, Packet, Peer, Vault},
};
use anyhow::{Context, Result, ensure};
use rustls::{ClientConnection, ServerConnection, StreamOwned, pki_types::ServerName};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::Child,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use zeroize::{Zeroize, Zeroizing};
const MAX_CONNECTIONS: usize = 4;
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    op: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    tagname: String,
    #[serde(default)]
    conversation: String,
    #[serde(default)]
    epoch: u64,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    pin: String,
}
impl Drop for Request {
    fn drop(&mut self) {
        self.text.zeroize();
        self.code.zeroize();
        self.tagname.zeroize();
    }
}
#[derive(Default)]
struct Slots {
    count: usize,
    peers: HashSet<[u8; 32]>,
}
pub struct Shared {
    vault: Mutex<Vault>,
    running: AtomicBool,
    lease: Mutex<Instant>,
    sockets: Mutex<HashMap<u64, TcpStream>>,
    socket_id: AtomicU64,
    killer: Mutex<Option<Arc<Mutex<Child>>>>,
    status: Mutex<&'static str>,
    redaction: AtomicU64,
    displayed: AtomicU64,
    generation: AtomicU64,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    slots: Mutex<Slots>,
    connections: Mutex<HashMap<String, &'static str>>,
    refresh: Mutex<HashSet<String>>,
    peer_count: AtomicUsize,
}
pub struct Session {
    pub shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    watchdog: Option<JoinHandle<()>>,
}
impl Shared {
    fn new(vault: Vault) -> Self {
        Self {
            vault: Mutex::new(vault),
            running: AtomicBool::new(true),
            lease: Mutex::new(Instant::now()),
            sockets: Mutex::new(HashMap::new()),
            socket_id: AtomicU64::new(0),
            killer: Mutex::new(None),
            status: Mutex::new("Conectando ao Tor"),
            redaction: AtomicU64::new(0),
            displayed: AtomicU64::new(0),
            generation: AtomicU64::new(0),
            locks: Mutex::new(HashMap::new()),
            slots: Mutex::new(Slots::default()),
            connections: Mutex::new(HashMap::new()),
            refresh: Mutex::new(HashSet::new()),
            peer_count: AtomicUsize::new(0),
        }
    }
    fn alive(&self) -> bool {
        if self.lease.lock().unwrap().elapsed() > Duration::from_secs(2) {
            self.revoke();
        }
        self.running.load(Ordering::SeqCst)
    }
    fn live_generation(&self, g: u64) -> bool {
        self.alive() && self.generation.load(Ordering::SeqCst) == g
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
    fn close_sockets(&self) {
        for socket in self.sockets.lock().unwrap().values() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
    pub fn revoke(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.close_sockets();
        if let Some(child) = self.killer.lock().unwrap().as_ref() {
            let _ = child.lock().unwrap().kill();
        }
    }
    fn conversation_lock(&self, cid: &str) -> Result<Arc<Mutex<()>>> {
        {
            let v = self.vault.lock().unwrap();
            if !v.is_group(cid)? {
                v.peer(cid)?;
            }
        }
        Ok(self
            .locks
            .lock()
            .unwrap()
            .entry(cid.into())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
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
        let mut selected = if req.conversation.is_empty() {
            None
        } else {
            Some(req.conversation.clone())
        };
        match req.op.as_str() {
            "pair" => {
                let mut v = self.vault.lock().unwrap();
                ensure!(self.alive(), "locked");
                selected = Some(v.pair_named(&req.code, &req.tagname)?);
            }
            "preview_group" => {
                return self
                    .vault
                    .lock()
                    .unwrap()
                    .preview_group(&req.code)
                    .map(|preview| json!({"preview":preview}));
            }
            "create_group" | "import_group" => {
                let mut v = self.vault.lock().unwrap();
                selected = Some(if req.op == "create_group" {
                    v.create_group(&req.tagname, &req.members)?
                } else {
                    v.import_group(&req.code)?
                });
            }
            "send" | "delete" | "rename" | "rename_member" => {
                let cid = selected
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(|| self.vault.lock().unwrap().only_conversation())?;
                let lock = self.conversation_lock(&cid)?;
                let _conversation = lock.lock().unwrap();
                let mut v = self.vault.lock().unwrap();
                ensure!(self.alive(), "locked");
                if v.is_group(&cid)? {
                    match req.op.as_str() {
                        "send" => v.group_send(&cid, &req.text)?,
                        "delete" => v.group_delete_sent(&cid)?,
                        "rename_member" => v.group_rename_member(
                            &cid,
                            hex::decode(&req.pin)?
                                .try_into()
                                .map_err(|_| anyhow::anyhow!("invalid member"))?,
                            &req.tagname,
                        )?,
                        _ => anyhow::bail!("group roster and title are fixed"),
                    }
                } else {
                    match req.op.as_str() {
                        "send" => v.send_to(&cid, &req.text)?,
                        "delete" => v.delete_sent(&cid)?,
                        "rename" => v.rename(&cid, &req.tagname)?,
                        _ => anyhow::bail!("not a group"),
                    }
                }
                selected = Some(cid);
            }
            "view" => {}
            _ => anyhow::bail!("unknown operation"),
        }
        let v = self.vault.lock().unwrap();
        ensure!(self.alive(), "locked");
        Ok(
            json!({"view":v.snapshot(selected.as_deref())?,"network":*self.status.lock().unwrap(),"connections":*self.connections.lock().unwrap(),"epoch":self.redaction.load(Ordering::SeqCst)}),
        )
    }
    fn apply(&self, pin: [u8; 32], packet: Packet, g: u64) -> Result<Option<Packet>> {
        let peer = self.vault.lock().unwrap().peer_by_pin(pin)?;
        if packet.version == 1 && matches!(packet.content, Content::Poll) {
            ensure!(
                packet.author == pin && hex::encode(packet.conversation) == peer.id,
                "invalid peer poll"
            );
            return Ok(None);
        }
        let cid = if packet.version == 2 {
            hex::encode(packet.conversation)
        } else {
            peer.id
        };
        // Unknown invitations may share an existing DM channel. Wait does not grant membership.
        let lock = self
            .locks
            .lock()
            .unwrap()
            .entry(cid.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let reply = {
            let _conversation = lock.lock().unwrap();
            let mut v = self.vault.lock().unwrap();
            ensure!(self.live_generation(g), "locked");
            if packet.version == 2 {
                v.apply_group(pin, packet)?
            } else {
                v.apply_from(pin, packet)?
            }
        };
        let deletion = reply.as_ref().is_some_and(|p| {
            matches!(
                p.content,
                Content::Deleted { .. }
                    | Content::Group {
                        event: GroupEvent::Deleted { .. }
                    }
            )
        });
        if deletion {
            let epoch = self.redaction.fetch_add(1, Ordering::SeqCst) + 1;
            while self.displayed.load(Ordering::SeqCst) < epoch {
                ensure!(self.live_generation(g), "locked");
                pause(self, 50)?;
            }
        }
        Ok(reply)
    }
    fn register<'a>(&'a self, socket: &TcpStream, g: u64) -> Result<SocketGuard<'a>> {
        let mut sockets = self.sockets.lock().unwrap();
        ensure!(
            self.running.load(Ordering::SeqCst) && self.generation.load(Ordering::SeqCst) == g,
            "locked"
        );
        let id = self.socket_id.fetch_add(1, Ordering::SeqCst);
        sockets.insert(id, socket.try_clone()?);
        Ok(SocketGuard { s: self, id })
    }
}
struct SocketGuard<'a> {
    s: &'a Shared,
    id: u64,
}
impl Drop for SocketGuard<'_> {
    fn drop(&mut self) {
        if let Some(socket) = self.s.sockets.lock().unwrap().remove(&self.id) {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}
struct Slot {
    shared: Arc<Shared>,
    pin: Option<[u8; 32]>,
    cid: Option<String>,
}
impl Slot {
    fn reserve(shared: &Arc<Shared>, peer: Option<&Peer>) -> Option<Self> {
        let mut slots = shared.slots.lock().unwrap();
        if slots.count >= MAX_CONNECTIONS || peer.is_some_and(|p| slots.peers.contains(&p.pin)) {
            return None;
        }
        slots.count += 1;
        if let Some(p) = peer {
            slots.peers.insert(p.pin);
        }
        Some(Self {
            shared: shared.clone(),
            pin: peer.map(|p| p.pin),
            cid: peer.map(|p| p.id.clone()),
        })
    }
    fn claim(&mut self, peer: &Peer) -> Result<()> {
        let mut slots = self.shared.slots.lock().unwrap();
        ensure!(!slots.peers.contains(&peer.pin), "duplicate connection");
        slots.peers.insert(peer.pin);
        self.pin = Some(peer.pin);
        self.cid = Some(peer.id.clone());
        Ok(())
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        let mut slots = self.shared.slots.lock().unwrap();
        slots.count -= 1;
        if let Some(pin) = self.pin {
            slots.peers.remove(&pin);
        }
        if let Some(cid) = &self.cid {
            self.shared
                .connections
                .lock()
                .unwrap()
                .insert(cid.clone(), "Offline");
        }
    }
}
impl Session {
    pub fn open(directory: &Path, binary: &Path, key: &[u8], create: bool) -> Result<Self> {
        ensure!(
            !directory.join("wipe.intent").exists() && !directory.join("setup.intent").exists(),
            "cleanup required"
        );
        std::fs::create_dir_all(directory)?;
        let shared = Arc::new(Shared::new(Vault::open(
            &directory.join("vault.db"),
            key,
            create,
        )?));
        let s = shared.clone();
        let directory = directory.to_owned();
        let binary = binary.to_owned();
        let worker = thread::spawn(move || {
            while s.alive() {
                let result = network(&s, &directory, &binary);
                #[cfg(all(feature = "probe-diagnostics", not(target_os = "android")))]
                if let Err(ref e) = result {
                    eprintln!("Test-fixture network stage: {e:?}");
                }
                let _ = result;
                *s.status.lock().unwrap() = "Tor indisponível — mensagens pendentes";
                if pause(&s, 2000).is_err() {
                    break;
                }
            }
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
        self.finish();
    }
    fn finish(&mut self) {
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
        self.finish();
    }
}
fn pause(s: &Shared, millis: u64) -> Result<()> {
    for _ in 0..millis.div_ceil(100) {
        ensure!(s.alive(), "locked");
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
struct Network {
    shared: Arc<Shared>,
    tor: Tor,
    workers: Vec<JoinHandle<()>>,
}
impl Drop for Network {
    fn drop(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.close_sockets();
        self.tor.stop();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
        self.shared.killer.lock().unwrap().take();
    }
}
fn network(s: &Arc<Shared>, dir: &Path, binary: &Path) -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let key = s
        .vault
        .lock()
        .unwrap()
        .get("onion-key")?
        .map(Zeroizing::new);
    let keystr = key.as_ref().map(|b| std::str::from_utf8(b)).transpose()?;
    let tor = Tor::start(
        binary,
        &dir.join("tor"),
        listener.local_addr()?.port(),
        keystr,
    )?;
    *s.killer.lock().unwrap() = Some(tor.child_handle());
    {
        let v = s.vault.lock().unwrap();
        ensure!(s.alive(), "locked");
        if let Some(old) = v.get("onion")? {
            ensure!(old == tor.onion.as_bytes(), "onion identity changed");
        }
        v.put("onion-key", tor.onion_key.as_bytes())?;
        v.put("onion", tor.onion.as_bytes())?;
    }
    let generation = s.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let mut net = Network {
        shared: s.clone(),
        tor,
        workers: Vec::new(),
    };
    ensure!(s.alive(), "locked");
    net.tor.enable_network()?;
    while s.alive() && !net.tor.bootstrapped()? {
        pause(s, 500)?;
    }
    *s.status.lock().unwrap() = "Tor pronto";
    let mut cursor = 0usize;
    let mut check = Instant::now();
    while s.alive() {
        let peers = s.vault.lock().unwrap().peers()?;
        s.peer_count.store(peers.len(), Ordering::SeqCst);
        let own = s.vault.lock().unwrap().identity.pin();
        let refresh: Vec<_> = s.refresh.lock().unwrap().drain().collect();
        for onion in refresh {
            let _ = net.tor.refresh_service(&onion);
        }
        net.workers.retain_mut(|w| !w.is_finished());
        // Incoming handshakes count toward the same four-slot bound as outbound attempts.
        if let Some(mut slot) = Slot::reserve(s, None) {
            match listener.accept() {
                Ok((socket, _)) => {
                    let config = {
                        let v = s.vault.lock().unwrap();
                        tls::server_config(&v.identity, peers.iter().map(|p| p.pin).collect())?
                    };
                    net.workers.push(thread::spawn(move || {
                        let shared = slot.shared.clone();
                        let result = (|| -> Result<()> {
                            let _socket = shared.register(&socket, generation)?;
                            configure(&socket)?;
                            let mut stream =
                                StreamOwned::new(ServerConnection::new(config)?, socket);
                            while stream.conn.is_handshaking() {
                                ensure!(shared.live_generation(generation), "locked");
                                stream.conn.complete_io(&mut stream.sock)?;
                            }
                            let pin = tls::fingerprint(
                                stream
                                    .conn
                                    .peer_certificates()
                                    .and_then(|c| c.first())
                                    .context("client certificate missing")?,
                            );
                            let peer = shared.vault.lock().unwrap().peer_by_pin(pin)?;
                            ensure!(own > pin, "unexpected connection direction");
                            slot.claim(&peer)?;
                            connected(&shared, &peer);
                            let groups = stream.conn.alpn_protocol() == Some(tls::GROUP_ALPN);
                            let mut scheduler = Scheduler::new(groups);
                            let started = Instant::now();
                            while keep_connection(&shared, generation, started) {
                                let mut acks = Vec::new();
                                for p in read_batch(&mut stream)? {
                                    scheduler.observe(&p)?;
                                    if let Some(ack) = shared.apply(pin, p, generation)? {
                                        ensure!(acks.is_empty(), "too many requests");
                                        acks.push(ack);
                                    }
                                }
                                write_batch(
                                    &mut stream,
                                    &acks,
                                    &shared,
                                    &peer,
                                    generation,
                                    &mut scheduler,
                                )?;
                            }
                            Ok(())
                        })();
                        report(&result);
                    }));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => drop(slot),
                Err(_) => anyhow::bail!("listener failed"),
            }
        }
        if !peers.is_empty() {
            let peer = &peers[cursor % peers.len()];
            let waiting = own < peer.pin && !s.slots.lock().unwrap().peers.contains(&peer.pin);
            if !waiting {
                cursor = cursor.wrapping_add(1);
            } else if let Some(slot) = Slot::reserve(s, Some(peer)) {
                cursor = cursor.wrapping_add(1);
                let config = {
                    let v = s.vault.lock().unwrap();
                    tls::configs(&v.identity, peer.pin)?.0
                };
                let peer = peer.clone();
                let proxy = net.tor.proxy;
                s.connections
                    .lock()
                    .unwrap()
                    .insert(peer.id.clone(), "Conectando");
                net.workers.push(thread::spawn(move || {
                    let slot = slot;
                    let result = dial(&slot.shared, &peer, proxy, config, generation);
                    if result.is_err() && slot.shared.live_generation(generation) {
                        slot.shared
                            .refresh
                            .lock()
                            .unwrap()
                            .insert(peer.onion.clone());
                        let _ = pause(&slot.shared, 2000);
                    }
                    report(&result);
                }));
            }
        }
        if check.elapsed() > Duration::from_secs(2) {
            ensure!(net.tor.bootstrapped()?, "Tor unavailable");
            check = Instant::now();
        }
        pause(s, 100)?;
    }
    Ok(())
}
fn report(result: &Result<()>) {
    #[cfg(all(feature = "probe-diagnostics", not(target_os = "android")))]
    if let Err(e) = result {
        eprintln!("Test-fixture peer stage: {e:?}");
    }
    let _ = result;
}
fn configure(socket: &TcpStream) -> Result<()> {
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(Duration::from_secs(30)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    Ok(())
}
fn connected(s: &Shared, p: &Peer) {
    s.connections
        .lock()
        .unwrap()
        .insert(p.id.clone(), "Conectado");
}
fn keep_connection(s: &Shared, g: u64, start: Instant) -> bool {
    s.live_generation(g)
        && (s.peer_count.load(Ordering::SeqCst) <= MAX_CONNECTIONS
            || start.elapsed() < Duration::from_secs(30))
}
fn dial(
    s: &Shared,
    peer: &Peer,
    proxy: SocketAddr,
    config: Arc<rustls::ClientConfig>,
    g: u64,
) -> Result<()> {
    let socket = transport::onion_stream(proxy, &peer.onion)?;
    let _socket = s.register(&socket, g)?;
    configure(&socket)?;
    let mut stream = StreamOwned::new(
        ClientConnection::new(config, ServerName::try_from("psychotalk.invalid")?)?,
        socket,
    );
    while stream.conn.is_handshaking() {
        ensure!(s.live_generation(g), "locked");
        stream.conn.complete_io(&mut stream.sock)?;
    }
    connected(s, peer);
    let mut scheduler = Scheduler::new(stream.conn.alpn_protocol() == Some(tls::GROUP_ALPN));
    let mut ack = None;
    let start = Instant::now();
    while keep_connection(s, g, start) {
        let mut batch = Vec::new();
        if let Some(p) = ack.take() {
            batch.push(p);
        }
        write_batch(&mut stream, &batch, s, peer, g, &mut scheduler)?;
        for p in read_batch(&mut stream)? {
            scheduler.observe(&p)?;
            if let Some(reply) = s.apply(peer.pin, p, g)? {
                ensure!(ack.is_none(), "too many requests");
                ack = Some(reply);
            }
        }
        pause(s, 500)?;
    }
    Ok(())
}
struct Scheduler {
    groups: bool,
    cursor: usize,
    deferred: HashMap<String, Instant>,
    outstanding: Option<String>,
}
impl Scheduler {
    fn new(groups: bool) -> Self {
        Self {
            groups,
            cursor: 0,
            deferred: HashMap::new(),
            outstanding: None,
        }
    }
    fn observe(&mut self, p: &Packet) -> Result<()> {
        ensure!(
            p.version != 2 || self.groups,
            "group protocol not negotiated"
        );
        if matches!(
            p.content,
            Content::Group {
                event: GroupEvent::Wait
            }
        ) {
            let gid = hex::encode(p.conversation);
            if self.outstanding.as_ref() == Some(&gid) {
                self.deferred.insert(gid, Instant::now());
            }
        }
        Ok(())
    }
}
fn priority(p: &Packet) -> u8 {
    match p.content {
        Content::Delete { .. }
        | Content::Group {
            event: GroupEvent::Delete { .. },
        } => 0,
        Content::Group {
            event: GroupEvent::Join | GroupEvent::Ready,
        } => 1,
        _ => 2,
    }
}
fn write_batch(
    stream: &mut impl Write,
    acks: &[Packet],
    s: &Shared,
    peer: &Peer,
    g: u64,
    scheduler: &mut Scheduler,
) -> Result<()> {
    let mut flows = Vec::new();
    {
        let v = s.vault.lock().unwrap();
        if v.peer(&peer.id).is_ok() {
            let p = v.next_for(&peer.id)?;
            if !matches!(p.content, Content::Poll) {
                flows.push((priority(&p), peer.id.clone(), false));
            }
        }
        if scheduler.groups {
            for gid in v.group_ids_for_peer(peer.pin)? {
                if scheduler
                    .deferred
                    .get(&gid)
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(30))
                {
                    continue;
                }
                if let Some(p) = v.group_next(&gid, peer.pin)? {
                    flows.push((priority(&p), gid, true));
                }
            }
        }
    }
    flows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let count = flows
        .first()
        .map_or(0, |f| flows.iter().take_while(|p| p.0 == f.0).count());
    let chosen = if count > 0 {
        let at = scheduler.cursor % count;
        scheduler.cursor = scheduler.cursor.wrapping_add(1);
        Some(&flows[at])
    } else {
        None
    };
    // Hold the selected conversation lock through I/O, but release the database first.
    // All fanout copies and author deletion share this lock for the group ID.
    let lock = chosen
        .map(|(_, cid, _)| s.conversation_lock(cid))
        .transpose()?;
    let _conversation = lock.as_ref().map(|l| l.lock().unwrap());
    ensure!(s.live_generation(g), "locked");
    let packet = {
        let v = s.vault.lock().unwrap();
        let next = match chosen {
            Some((_, cid, true)) => v.group_next(cid, peer.pin)?,
            Some((_, cid, false)) => Some(v.next_for(cid)?),
            None => None,
        };
        next.unwrap_or(Packet {
            version: 1,
            conversation: hex::decode(&peer.id)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid peer"))?,
            author: v.identity.pin(),
            content: Content::Poll,
        })
    };
    scheduler.outstanding = if packet.version == 2 {
        Some(hex::encode(packet.conversation))
    } else {
        None
    };
    let mut batch = acks.to_vec();
    batch.push(packet);
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
            .filter(|p| match &p.content {
                Content::Message { .. } | Content::Delete { .. } | Content::Poll => true,
                Content::Group { event } => event.is_request(),
                _ => false,
            })
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
        let v = Vault::open(&root.path().join("vault"), &[9; 32], true).unwrap();
        (root, Shared::new(v))
    }
    #[test]
    fn expired_heartbeat_cannot_resurrect_session() {
        let (_r, s) = fixture();
        *s.lease.lock().unwrap() = Instant::now() - Duration::from_secs(3);
        s.heartbeat();
        assert!(!s.running.load(Ordering::SeqCst));
        assert!(s.command("{\"op\":\"view\"}").is_err());
    }
    #[test]
    fn four_slots_include_handshakes_and_duplicate_peer_does_not_release_another() {
        let (_root, s) = fixture();
        let shared = Arc::new(s);
        let peer = Peer {
            id: "test-peer".into(),
            onion: String::new(),
            tagname: "fixture".into(),
            pin: [42; 32],
        };
        let first = Slot::reserve(&shared, Some(&peer)).unwrap();
        let mut pending = Slot::reserve(&shared, None).unwrap();
        assert!(pending.claim(&peer).is_err());
        drop(pending);
        assert!(shared.slots.lock().unwrap().peers.contains(&peer.pin));
        let rest: Vec<_> = (0..3)
            .map(|_| Slot::reserve(&shared, None).unwrap())
            .collect();
        assert_eq!(shared.slots.lock().unwrap().count, MAX_CONNECTIONS);
        assert!(Slot::reserve(&shared, None).is_none());
        drop(rest);
        drop(first);
        assert_eq!(shared.slots.lock().unwrap().count, 0);
    }
    #[test]
    fn blocked_writer_holds_only_its_conversation_not_the_database() {
        use sha3::{Digest, Sha3_256};
        use std::sync::mpsc;
        let (root, s) = fixture();
        let mut ids = Vec::new();
        for n in [42u8, 43] {
            let key = [n; 32];
            let mut h = Sha3_256::new();
            h.update(b".onion checksum");
            h.update(key);
            h.update([3]);
            let mut raw = key.to_vec();
            raw.extend_from_slice(&h.finalize()[..2]);
            raw.push(3);
            let code = crate::contact::Contact {
                onion: format!(
                    "{}.onion",
                    data_encoding::BASE32_NOPAD
                        .encode(&raw)
                        .to_ascii_lowercase()
                ),
                pin: key,
            }
            .code();
            ids.push(s.vault.lock().unwrap().pair(&code).unwrap());
        }
        s.vault.lock().unwrap().send_to(&ids[0], "queued").unwrap();
        let shared = Arc::new(s);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        struct BlockedWriter {
            entered: Option<mpsc::Sender<()>>,
            release: mpsc::Receiver<()>,
        }
        impl Write for BlockedWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                if let Some(sender) = self.entered.take() {
                    sender.send(()).unwrap();
                    self.release.recv().unwrap();
                }
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let writer = shared.clone();
        let a = shared.vault.lock().unwrap().peer(&ids[0]).unwrap();
        let sending = thread::spawn(move || {
            write_batch(
                &mut BlockedWriter {
                    entered: Some(entered_tx),
                    release: release_rx,
                },
                &[],
                &writer,
                &a,
                0,
                &mut Scheduler::new(true),
            )
            .unwrap()
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let (deleted_tx, deleted_rx) = mpsc::channel();
        let deleting = shared.clone();
        let a = ids[0].clone();
        let deletion = thread::spawn(move || {
            deleting
                .command(&json!({"op":"delete","conversation":a}).to_string())
                .unwrap();
            deleted_tx.send(()).unwrap();
        });
        let other = shared
            .command(&json!({"op":"send","conversation":ids[1],"text":"independent"}).to_string())
            .unwrap();
        assert_eq!(
            other["view"]["active"]["messages"][0]["text"],
            "independent"
        );
        assert!(deleted_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        sending.join().unwrap();
        deleted_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        deletion.join().unwrap();
        drop(root);
    }
}
