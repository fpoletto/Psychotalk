use crate::{contact::Contact, tls::Identity};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use rand::{RngCore, rngs::OsRng};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use zeroize::{Zeroize, Zeroizing};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Packet {
    pub version: u8,
    pub conversation: [u8; 32],
    pub author: [u8; 32],
    pub content: Content,
}
impl Drop for Packet {
    fn drop(&mut self) {
        match &mut self.content {
            Content::Message { body, .. }
            | Content::Group {
                event: crate::groups::GroupEvent::Text { body, .. },
            } => body.zeroize(),
            _ => {}
        }
    }
}
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Content {
    Group { event: crate::groups::GroupEvent },
    Message { id: String, seq: u64, body: String },
    Delivered { id: String, seq: u64 },
    Delete { id: String, limit: u64 },
    Deleted { id: String, limit: u64 },
    Poll,
}
#[derive(Serialize)]
pub struct Message {
    pub mine: bool,
    pub text: String,
    pub delivered: bool,
    pub author: String,
    pub delivered_count: usize,
    pub recipient_count: usize,
}
impl Drop for Message {
    fn drop(&mut self) {
        self.text.zeroize();
    }
}
#[derive(Serialize)]
pub struct View {
    pub contact: bool,
    pub own_code: Option<String>,
    pub messages: Vec<Message>,
    pub deletion: Option<bool>,
}
pub struct Vault {
    pub(crate) db: Connection,
    pub identity: Identity,
}
fn id() -> String {
    let mut b = [0; 16];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}
fn valid_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn number(n: u64) -> Result<i64> {
    ensure!(n <= i64::MAX as u64, "invalid sequence");
    Ok(n as i64)
}

#[derive(Clone, Serialize)]
pub struct Peer {
    pub id: String,
    pub tagname: String,
    pub onion: String,
    pub pin: [u8; 32],
}
#[derive(Serialize)]
pub struct Conversation {
    pub id: String,
    pub tagname: String,
    pub fingerprint: String,
    pub pending: i64,
    pub deletion_pending: bool,
    pub kind: String,
}
#[derive(Serialize)]
pub struct Chat {
    pub kind: String,
    pub group: Option<serde_json::Value>,
    pub id: String,
    pub tagname: String,
    pub messages: Vec<Message>,
    pub deletion: Option<bool>,
}
#[derive(Serialize)]
pub struct Snapshot {
    pub own_code: Option<String>,
    pub conversations: Vec<Conversation>,
    pub active: Option<Chat>,
}
pub fn conversation_id(a: [u8; 32], b: [u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"Psychotalk conversation v1");
    hash.update(a.min(b));
    hash.update(a.max(b));
    hash.finalize().into()
}
pub(crate) fn display_name(alias: String, pin: &[u8]) -> String {
    if alias.is_empty() {
        format!("Contato {}", &hex::encode(pin)[..8])
    } else {
        alias
    }
}
const SCHEMA:&str="
 CREATE TABLE contacts(pin BLOB PRIMARY KEY CHECK(length(pin)=32),onion TEXT NOT NULL UNIQUE,tagname TEXT NOT NULL DEFAULT '');
 CREATE TABLE conversations(id TEXT PRIMARY KEY,peer BLOB NOT NULL UNIQUE REFERENCES contacts(pin),activity INTEGER NOT NULL DEFAULT 0);
 CREATE TABLE counters(conversation TEXT NOT NULL REFERENCES conversations(id),author INTEGER NOT NULL,seq INTEGER NOT NULL,watermark INTEGER NOT NULL,PRIMARY KEY(conversation,author));
 CREATE TABLE messages(conversation TEXT NOT NULL REFERENCES conversations(id),id TEXT NOT NULL,author INTEGER NOT NULL,seq INTEGER NOT NULL,body BLOB NOT NULL,delivered INTEGER NOT NULL,PRIMARY KEY(conversation,id),UNIQUE(conversation,author,seq));
 CREATE TABLE deletions(conversation TEXT NOT NULL REFERENCES conversations(id),id TEXT NOT NULL,lim INTEGER NOT NULL,acked INTEGER NOT NULL,PRIMARY KEY(conversation,id));";
fn touch(db: &Connection, cid: &str) -> Result<()> {
    let has_groups: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='groups')",
        [],
        |r| r.get(0),
    )?;
    if has_groups {
        db.execute("UPDATE conversations SET activity=1+max((SELECT COALESCE(MAX(activity),0) FROM conversations),(SELECT COALESCE(MAX(activity),0) FROM groups)) WHERE id=?",[cid])?;
    } else {
        db.execute("UPDATE conversations SET activity=(SELECT COALESCE(MAX(activity),0)+1 FROM conversations) WHERE id=?",[cid])?;
    }
    Ok(())
}
fn insert_peer(db: &Connection, own: [u8; 32], contact: &Contact, alias: &str) -> Result<String> {
    let cid = hex::encode(conversation_id(own, contact.pin));
    db.execute(
        "INSERT INTO contacts(pin,onion,tagname) VALUES(?,?,?) ON CONFLICT(pin) DO NOTHING",
        params![contact.pin.as_slice(), contact.onion, alias],
    )?;
    let existing: String = db.query_row(
        "SELECT onion FROM contacts WHERE pin=?",
        [contact.pin.as_slice()],
        |r| r.get(0),
    )?;
    ensure!(existing == contact.onion, "endpoint conflict");
    if !alias.is_empty() {
        db.execute(
            "UPDATE contacts SET tagname=? WHERE pin=?",
            params![alias, contact.pin.as_slice()],
        )?;
    }
    db.execute(
        "INSERT INTO conversations(id,peer) VALUES(?,?)",
        params![cid, contact.pin.as_slice()],
    )?;
    db.execute(
        "INSERT INTO counters VALUES(?,0,0,0),(?,1,0,0)",
        params![cid, cid],
    )?;
    touch(db, &cid)?;
    Ok(cid)
}
fn migrate(db: &mut Connection, identity: &Identity) -> Result<()> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(
        version == 0 || version == 2 || version == 3,
        "unsupported vault version"
    );
    if version == 2 || version == 3 {
        return Ok(());
    }
    let legacy: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='messages')",
        [],
        |r| r.get(0),
    )?;
    let tx = db.transaction()?;
    if legacy {
        tx.execute_batch("ALTER TABLE messages RENAME TO legacy_messages; ALTER TABLE counters RENAME TO legacy_counters; ALTER TABLE deletions RENAME TO legacy_deletions;")?;
    }
    tx.execute_batch(SCHEMA)?;
    if legacy {
        let contact: Option<Vec<u8>> = tx
            .query_row("SELECT v FROM config WHERE k='contact'", [], |r| r.get(0))
            .optional()?;
        if let Some(bytes) = contact {
            let contact: Contact = serde_json::from_slice(&bytes)?;
            Contact::parse(&contact.code())?;
            ensure!(contact.pin != identity.pin(), "invalid legacy identity");
            let cid = insert_peer(&tx, identity.pin(), &contact, "")?;
            tx.execute("INSERT OR REPLACE INTO counters SELECT ?,author,seq,watermark FROM legacy_counters",[&cid])?;
            tx.execute("INSERT INTO messages(rowid,conversation,id,author,seq,body,delivered) SELECT rowid,?,id,author,seq,body,delivered FROM legacy_messages",[&cid])?;
            tx.execute("INSERT INTO deletions(rowid,conversation,id,lim,acked) SELECT rowid,?,id,lim,acked FROM legacy_deletions",[&cid])?;
        } else {
            let count:i64=tx.query_row("SELECT (SELECT count(*) FROM legacy_messages)+(SELECT count(*) FROM legacy_deletions)+(SELECT count(*) FROM legacy_counters WHERE seq<>0 OR watermark<>0)",[],|r|r.get(0))?;
            ensure!(count == 0, "legacy contact missing; migration rolled back");
        }
        tx.execute_batch("DROP TABLE legacy_messages; DROP TABLE legacy_counters; DROP TABLE legacy_deletions; DELETE FROM config WHERE k='contact';")?;
    }
    ensure!(
        tx.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))? == 0,
        "migration consistency failed"
    );
    tx.pragma_update(None, "user_version", 2)?;
    tx.commit()?;
    Ok(())
}

impl Vault {
    pub fn open(path: &Path, key: &[u8], create: bool) -> Result<Self> {
        ensure!(
            key.len() == 32 && (create || path.is_file()),
            "invalid vault"
        );
        let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | if create {
                rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            } else {
                rusqlite::OpenFlags::empty()
            };
        let mut db = Connection::open_with_flags(path, flags)?;
        let command = Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", hex::encode(key)));
        db.execute_batch(&command)?;
        let version: String = db.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
        ensure!(!version.is_empty(), "SQLCipher required");
        db.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })?;
        db.execute_batch("PRAGMA cipher_memory_security=ON; PRAGMA secure_delete=ON; PRAGMA temp_store=MEMORY; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS config(k TEXT PRIMARY KEY,v BLOB NOT NULL);")?;
        let bytes: Option<Vec<u8>> = db
            .query_row("SELECT v FROM config WHERE k='identity'", [], |r| r.get(0))
            .optional()?;
        let identity = match bytes {
            Some(bytes) => serde_json::from_slice(&Zeroizing::new(bytes))?,
            None => {
                ensure!(create, "missing identity");
                let identity = Identity::generate()?;
                let bytes = Zeroizing::new(serde_json::to_vec(&identity)?);
                db.execute(
                    "INSERT INTO config VALUES('identity',?)",
                    [bytes.as_slice()],
                )?;
                identity
            }
        };
        migrate(&mut db, &identity)?;
        crate::groups::migrate(&mut db)?;
        Ok(Self { db, identity })
    }
    pub fn put(&self, k: &str, v: &[u8]) -> Result<()> {
        self.db.execute(
            "INSERT INTO config VALUES(?,?) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
            params![k, v],
        )?;
        Ok(())
    }
    pub fn get(&self, k: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .query_row("SELECT v FROM config WHERE k=?", [k], |r| r.get(0))
            .optional()?)
    }
    pub fn direct_peers(&self) -> Result<Vec<Peer>> {
        let mut stmt=self.db.prepare("SELECT c.id,p.onion,p.pin,p.tagname FROM conversations c JOIN contacts p ON p.pin=c.peer ORDER BY c.id")?;
        Ok(stmt
            .query_map([], |r| {
                let pin: Vec<u8> = r.get(2)?;
                let alias: String = r.get(3)?;
                Ok(Peer {
                    id: r.get(0)?,
                    onion: r.get(1)?,
                    tagname: display_name(alias, &pin),
                    pin: pin.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn peers(&self) -> Result<Vec<Peer>> {
        let mut peers = self.direct_peers()?;
        for p in self.group_peers()? {
            if !peers.iter().any(|known| known.pin == p.pin) {
                peers.push(p);
            }
        }
        peers.sort_by_key(|p| p.pin);
        Ok(peers)
    }
    pub fn peer(&self, cid: &str) -> Result<Peer> {
        self.direct_peers()?
            .into_iter()
            .find(|p| p.id == cid)
            .context("unknown conversation")
    }
    pub fn peer_by_pin(&self, pin: [u8; 32]) -> Result<Peer> {
        self.peers()?
            .into_iter()
            .find(|p| p.pin == pin)
            .context("unregistered peer")
    }
    pub fn only_conversation(&self) -> Result<String> {
        let peers = self.direct_peers()?;
        ensure!(peers.len() == 1, "explicit conversation required");
        Ok(peers[0].id.clone())
    }
    pub fn pair(&mut self, code: &str) -> Result<String> {
        self.pair_named(code, "")
    }
    pub fn pair_named(&mut self, code: &str, alias: &str) -> Result<String> {
        let alias = alias.trim();
        ensure!(
            alias.chars().count() <= 64 && !alias.chars().any(char::is_control),
            "invalid tagname"
        );
        let contact = Contact::parse(code)?;
        ensure!(contact.pin != self.identity.pin(), "cannot pair with self");
        let tx = self.db.transaction()?;
        let cid = insert_peer(&tx, self.identity.pin(), &contact, alias)?;
        tx.commit()?;
        Ok(cid)
    }
    pub fn rename(&self, cid: &str, tagname: &str) -> Result<()> {
        let tagname = tagname.trim();
        ensure!(
            tagname.chars().count() <= 64 && !tagname.chars().any(char::is_control),
            "invalid tagname"
        );
        let peer = self.peer(cid)?;
        self.db.execute(
            "UPDATE contacts SET tagname=? WHERE pin=?",
            params![tagname, peer.pin.as_slice()],
        )?;
        Ok(())
    }
    pub fn packet_for(&self, cid: &str, content: Content) -> Result<Packet> {
        let peer = self.peer(cid)?;
        Ok(Packet {
            version: 1,
            conversation: conversation_id(self.identity.pin(), peer.pin),
            author: self.identity.pin(),
            content,
        })
    }
    pub fn send_to(&mut self, cid: &str, text: &str) -> Result<()> {
        self.peer(cid)?;
        ensure!(!text.is_empty() && text.len() <= 4096, "invalid message");
        let tx = self.db.transaction()?;
        let seq: i64 = tx.query_row(
            "SELECT seq FROM counters WHERE conversation=? AND author=0",
            [cid],
            |r| r.get(0),
        )?;
        let seq = seq.checked_add(1).context("sequence exhausted")?;
        tx.execute(
            "UPDATE counters SET seq=? WHERE conversation=? AND author=0",
            params![seq, cid],
        )?;
        tx.execute(
            "INSERT INTO messages VALUES(?,?,0,?,?,0)",
            params![cid, id(), seq, text.as_bytes()],
        )?;
        touch(&tx, cid)?;
        tx.commit()?;
        Ok(())
    }
    pub fn delete_sent(&mut self, cid: &str) -> Result<()> {
        self.peer(cid)?;
        let tx = self.db.transaction()?;
        let lim: i64 = tx.query_row(
            "SELECT seq FROM counters WHERE conversation=? AND author=0",
            [cid],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE counters SET watermark=max(watermark,?) WHERE conversation=? AND author=0",
            params![lim, cid],
        )?;
        tx.execute(
            "DELETE FROM messages WHERE conversation=? AND author=0 AND seq<=?",
            params![cid, lim],
        )?;
        tx.execute(
            "INSERT INTO deletions VALUES(?,?,?,0)",
            params![cid, id(), lim],
        )?;
        touch(&tx, cid)?;
        tx.commit()?;
        Ok(())
    }
    pub fn next_for(&self, cid: &str) -> Result<Packet> {
        let delete:Option<(String,i64)>=self.db.query_row("SELECT id,lim FROM deletions WHERE conversation=? AND acked=0 ORDER BY lim DESC,rowid DESC LIMIT 1",[cid],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((id, limit)) = delete {
            return self.packet_for(
                cid,
                Content::Delete {
                    id,
                    limit: limit as u64,
                },
            );
        }
        let message:Option<(String,i64,Vec<u8>)>=self.db.query_row("SELECT id,seq,body FROM messages WHERE conversation=? AND author=0 AND delivered=0 AND seq>(SELECT watermark FROM counters WHERE conversation=? AND author=0) ORDER BY seq LIMIT 1",params![cid,cid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        self.packet_for(
            cid,
            match message {
                Some((id, seq, body)) => Content::Message {
                    id,
                    seq: seq as u64,
                    body: STANDARD.encode(Zeroizing::new(body)),
                },
                None => Content::Poll,
            },
        )
    }
    pub fn apply_from(
        &mut self,
        authenticated: [u8; 32],
        mut packet: Packet,
    ) -> Result<Option<Packet>> {
        let peer = self.peer_by_pin(authenticated)?;
        let cid = peer.id;
        self.peer(&cid)?;
        ensure!(
            packet.version == 1
                && packet.author == authenticated
                && packet.conversation == conversation_id(self.identity.pin(), authenticated),
            "invalid authenticated context"
        );
        let reply = match std::mem::replace(&mut packet.content, Content::Poll) {
            Content::Message { id, seq, body } => {
                ensure!(
                    valid_id(&id) && seq > 0 && body.len() <= 5464,
                    "invalid message"
                );
                let seq = number(seq)?;
                let encoded = Zeroizing::new(body);
                let body = Zeroizing::new(STANDARD.decode(encoded.as_bytes())?);
                ensure!(
                    !body.is_empty() && body.len() <= 4096 && std::str::from_utf8(&body).is_ok(),
                    "invalid text"
                );
                let tx = self.db.transaction()?;
                let watermark: i64 = tx.query_row(
                    "SELECT watermark FROM counters WHERE conversation=? AND author=1",
                    [&cid],
                    |r| r.get(0),
                )?;
                if seq > watermark {
                    let existing:Option<(String,Vec<u8>)>=tx.query_row("SELECT id,body FROM messages WHERE conversation=? AND author=1 AND seq=?",params![cid,seq],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                    if let Some((old, bytes)) = existing {
                        ensure!(
                            old == id && Zeroizing::new(bytes).as_slice() == body.as_slice(),
                            "conflicting retransmission"
                        );
                    } else {
                        tx.execute(
                            "INSERT INTO messages VALUES(?,?,1,?,?,1)",
                            params![cid, id, seq, body.as_slice()],
                        )?;
                        touch(&tx, &cid)?;
                    }
                }
                tx.commit()?;
                Some(Content::Delivered {
                    id,
                    seq: seq as u64,
                })
            }
            Content::Delete { id, limit } => {
                ensure!(valid_id(&id), "invalid operation");
                let limit = number(limit)?;
                let tx = self.db.transaction()?;
                tx.execute("UPDATE counters SET watermark=max(watermark,?) WHERE conversation=? AND author=1",params![limit,cid])?;
                tx.execute("DELETE FROM messages WHERE conversation=? AND author=1 AND seq<=(SELECT watermark FROM counters WHERE conversation=? AND author=1)",params![cid,cid])?;
                touch(&tx, &cid)?;
                tx.commit()?;
                Some(Content::Deleted {
                    id,
                    limit: limit as u64,
                })
            }
            Content::Delivered { id, seq } => {
                self.db.execute("UPDATE messages SET delivered=1 WHERE conversation=? AND author=0 AND id=? AND seq=?",params![cid,id,number(seq)?])?;
                None
            }
            Content::Deleted { id, limit } => {
                let tx = self.db.transaction()?;
                tx.execute("UPDATE deletions SET acked=1 WHERE conversation=? AND lim<=? AND rowid<=(SELECT rowid FROM deletions WHERE conversation=? AND id=? AND lim=?)",params![cid,number(limit)?,cid,id,number(limit)?])?;
                tx.commit()?;
                None
            }
            Content::Poll => None,
            Content::Group { .. } => anyhow::bail!("group requires negotiated protocol v2"),
        };
        reply.map(|c| self.packet_for(&cid, c)).transpose()
    }
    pub fn chat(&self, cid: &str) -> Result<Chat> {
        let peer = self.peer(cid)?;
        let mut stmt = self.db.prepare(
            "SELECT author,body,delivered FROM messages WHERE conversation=? ORDER BY rowid",
        )?;
        let messages = stmt
            .query_map([cid], |r| {
                let body: Vec<u8> = r.get(1)?;
                Ok(Message {
                    mine: r.get::<_, i64>(0)? == 0,
                    text: String::from_utf8_lossy(&Zeroizing::new(body)).into_owned(),
                    delivered: r.get::<_, i64>(2)? == 1,
                    author: if r.get::<_, i64>(0)? == 0 {
                        "Você".into()
                    } else {
                        peer.tagname.clone()
                    },
                    delivered_count: if r.get::<_, i64>(2)? == 1 { 1 } else { 0 },
                    recipient_count: 1,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let deletion = self
            .db
            .query_row(
                "SELECT acked FROM deletions WHERE conversation=? ORDER BY rowid DESC LIMIT 1",
                [cid],
                |r| r.get(0),
            )
            .optional()?;
        Ok(Chat {
            kind: "direct".into(),
            group: None,
            id: cid.into(),
            tagname: peer.tagname,
            messages,
            deletion,
        })
    }
    pub fn snapshot(&self, selected: Option<&str>) -> Result<Snapshot> {
        let mut stmt=self.db.prepare("SELECT c.id,p.tagname,p.pin,(SELECT count(*) FROM messages m WHERE m.conversation=c.id AND m.author=0 AND m.delivered=0),EXISTS(SELECT 1 FROM deletions d WHERE d.conversation=c.id AND d.acked=0) FROM conversations c JOIN contacts p ON p.pin=c.peer ORDER BY c.activity DESC,c.id")?;
        let mut conversations: Vec<Conversation> = stmt
            .query_map([], |r| {
                let pin: Vec<u8> = r.get(2)?;
                Ok(Conversation {
                    id: r.get(0)?,
                    tagname: display_name(r.get(1)?, &pin),
                    fingerprint: hex::encode(pin),
                    pending: r.get(3)?,
                    deletion_pending: r.get(4)?,
                    kind: "direct".into(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let group_rows = self.group_summaries()?;
        let mut activities = std::collections::HashMap::new();
        {
            let mut q = self.db.prepare("SELECT id,activity FROM conversations")?;
            for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (id, activity) = row?;
                activities.insert(id, activity);
            }
        }
        for (row, activity) in group_rows {
            activities.insert(row.id.clone(), activity);
            conversations.push(row);
        }
        conversations.sort_by(|a, b| {
            activities
                .get(&b.id)
                .cmp(&activities.get(&a.id))
                .then(a.id.cmp(&b.id))
        });
        let own_code = self.get("onion")?.map(|s| {
            Contact {
                onion: String::from_utf8_lossy(&s).into_owned(),
                pin: self.identity.pin(),
            }
            .code()
        });
        Ok(Snapshot {
            own_code,
            conversations,
            active: selected
                .map(|cid| {
                    if self.is_group(cid)? {
                        self.group_chat(cid)
                    } else {
                        self.chat(cid)
                    }
                })
                .transpose()?,
        })
    }
    // Narrow compatibility helpers reject ambiguous access once there is more than one conversation.
    pub fn contact(&self) -> Result<Option<Contact>> {
        let peers = self.direct_peers()?;
        ensure!(peers.len() <= 1, "explicit conversation required");
        Ok(peers.into_iter().next().map(|p| Contact {
            onion: p.onion,
            pin: p.pin,
        }))
    }
    pub fn packet(&self, c: Content) -> Result<Packet> {
        self.packet_for(&self.only_conversation()?, c)
    }
    pub fn send(&mut self, text: &str) -> Result<()> {
        self.send_to(&self.only_conversation()?, text)
    }
    pub fn delete_all(&mut self) -> Result<()> {
        self.delete_sent(&self.only_conversation()?)
    }
    pub fn next(&self) -> Result<Packet> {
        self.next_for(&self.only_conversation()?)
    }
    pub fn apply(&mut self, p: Packet) -> Result<Option<Packet>> {
        let pin = self.contact()?.context("no contact")?.pin;
        self.apply_from(pin, p)
    }
    pub fn view(&self) -> Result<View> {
        let peers = self.direct_peers()?;
        ensure!(peers.len() <= 1, "explicit conversation required");
        let chat = peers.first().map(|p| self.chat(&p.id)).transpose()?;
        Ok(View {
            contact: !peers.is_empty(),
            own_code: self.snapshot(None)?.own_code,
            deletion: chat.as_ref().and_then(|c| c.deletion),
            messages: chat
                .map(|mut c| std::mem::take(&mut c.messages))
                .unwrap_or_default(),
        })
    }
}
pub fn derive(password: &[u8], salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    ensure!(
        password.len() <= 1024 && salt.len() == 16,
        "invalid KDF input"
    );
    let params = argon2::Params::new(65536, 3, 1, Some(32))
        .map_err(|_| anyhow::anyhow!("KDF parameters"))?;
    let mut key = Zeroizing::new([0; 32]);
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|_| anyhow::anyhow!("KDF failed"))?;
    Ok(key)
}
