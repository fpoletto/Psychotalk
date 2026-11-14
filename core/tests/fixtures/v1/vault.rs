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
        if let Content::Message { body, .. } = &mut self.content {
            body.zeroize();
        }
    }
}
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Content {
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
    db: Connection,
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
        let db = Connection::open_with_flags(path, flags)?;
        // A blob literal selects SQLCipher's binary key mode, bypassing its passphrase KDF.
        let command = Zeroizing::new(format!("PRAGMA key = \"x'{}'\";", hex::encode(key)));
        db.execute_batch(&command)?;
        let version: String = db.query_row("PRAGMA cipher_version", [], |r| r.get(0))?;
        ensure!(!version.is_empty(), "SQLCipher required");
        db.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        })?;
        db.execute_batch("PRAGMA cipher_memory_security=ON; PRAGMA secure_delete=ON; PRAGMA temp_store=MEMORY; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS config(k TEXT PRIMARY KEY,v BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS counters(author INTEGER PRIMARY KEY,seq INTEGER NOT NULL,watermark INTEGER NOT NULL);
            INSERT OR IGNORE INTO counters VALUES(0,0,0),(1,0,0);
            CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,author INTEGER NOT NULL,seq INTEGER NOT NULL,body BLOB NOT NULL,delivered INTEGER NOT NULL,UNIQUE(author,seq));
            CREATE TABLE IF NOT EXISTS deletions(id TEXT PRIMARY KEY,lim INTEGER NOT NULL,acked INTEGER NOT NULL);")?;
        let stored: Option<Vec<u8>> = db
            .query_row("SELECT v FROM config WHERE k='identity'", [], |r| r.get(0))
            .optional()?;
        let identity = match stored {
            Some(v) => serde_json::from_slice(&Zeroizing::new(v))?,
            None => {
                ensure!(create, "missing identity");
                let i = Identity::generate()?;
                let b = Zeroizing::new(serde_json::to_vec(&i)?);
                db.execute("INSERT INTO config VALUES('identity',?)", [b.as_slice()])?;
                i
            }
        };
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
    pub fn contact(&self) -> Result<Option<Contact>> {
        self.get("contact")?
            .map(|v| serde_json::from_slice(&v).map_err(Into::into))
            .transpose()
    }
    pub fn pair(&self, code: &str) -> Result<()> {
        ensure!(self.contact()?.is_none(), "contact already paired");
        let c = Contact::parse(code)?;
        ensure!(c.pin != self.identity.pin(), "cannot pair with self");
        self.put("contact", &serde_json::to_vec(&c)?)
    }
    fn conversation(&self) -> Result<[u8; 32]> {
        let a = self.identity.pin();
        let b = self.contact()?.context("no contact")?.pin;
        let mut h = Sha256::new();
        h.update(b"Psychotalk conversation v1");
        h.update(a.min(b));
        h.update(a.max(b));
        Ok(h.finalize().into())
    }
    pub fn packet(&self, content: Content) -> Result<Packet> {
        Ok(Packet {
            version: 1,
            conversation: self.conversation()?,
            author: self.identity.pin(),
            content,
        })
    }
    pub fn send(&mut self, text: &str) -> Result<()> {
        ensure!(
            self.contact()?.is_some() && !text.is_empty() && text.len() <= 4096,
            "invalid message"
        );
        let tx = self.db.transaction()?;
        let seq: i64 = tx.query_row("SELECT seq FROM counters WHERE author=0", [], |r| r.get(0))?;
        let next = seq.checked_add(1).context("sequence exhausted")?;
        tx.execute("UPDATE counters SET seq=? WHERE author=0", [next])?;
        tx.execute(
            "INSERT INTO messages VALUES(?,0,?,?,0)",
            params![id(), next, text.as_bytes()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn delete_all(&mut self) -> Result<()> {
        ensure!(self.contact()?.is_some(), "no contact");
        let tx = self.db.transaction()?;
        let lim: i64 = tx.query_row("SELECT seq FROM counters WHERE author=0", [], |r| r.get(0))?;
        tx.execute(
            "UPDATE counters SET watermark=max(watermark,?) WHERE author=0",
            [lim],
        )?;
        tx.execute("DELETE FROM messages WHERE author=0 AND seq<=?", [lim])?;
        tx.execute("INSERT INTO deletions VALUES(?,?,0)", params![id(), lim])?;
        tx.commit()?;
        Ok(())
    }
    pub fn next(&self) -> Result<Packet> {
        let delete: Option<(String, i64)> = self
            .db
            .query_row(
                "SELECT id,lim FROM deletions WHERE acked=0 ORDER BY lim DESC,rowid DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, limit)) = delete {
            return self.packet(Content::Delete {
                id,
                limit: limit as u64,
            });
        }
        let message:Option<(String,i64,Vec<u8>)>=self.db.query_row("SELECT id,seq,body FROM messages WHERE author=0 AND delivered=0 AND seq>(SELECT watermark FROM counters WHERE author=0) ORDER BY seq LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        self.packet(match message {
            Some((id, seq, body)) => Content::Message {
                id,
                seq: seq as u64,
                body: STANDARD.encode(Zeroizing::new(body)),
            },
            None => Content::Poll,
        })
    }
    pub fn apply(&mut self, mut p: Packet) -> Result<Option<Packet>> {
        ensure!(
            p.version == 1
                && p.conversation == self.conversation()?
                && p.author == self.contact()?.context("no contact")?.pin,
            "invalid authenticated context"
        );
        let reply = match std::mem::replace(&mut p.content, Content::Poll) {
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
                let watermark: i64 =
                    tx.query_row("SELECT watermark FROM counters WHERE author=1", [], |r| {
                        r.get(0)
                    })?;
                if seq > watermark {
                    let existing: Option<(String, Vec<u8>)> = tx
                        .query_row(
                            "SELECT id,body FROM messages WHERE author=1 AND seq=?",
                            [seq],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if let Some((old, bytes)) = existing {
                        ensure!(
                            old == id && Zeroizing::new(bytes).as_slice() == body.as_slice(),
                            "conflicting retransmission"
                        );
                    } else {
                        tx.execute(
                            "INSERT INTO messages VALUES(?,1,?,?,1)",
                            params![id, seq, body.as_slice()],
                        )?;
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
                tx.execute(
                    "UPDATE counters SET watermark=max(watermark,?) WHERE author=1",
                    [limit],
                )?;
                tx.execute("DELETE FROM messages WHERE author=1 AND seq<=(SELECT watermark FROM counters WHERE author=1)",[])?;
                tx.commit()?;
                Some(Content::Deleted {
                    id,
                    limit: limit as u64,
                })
            }
            Content::Delivered { id, seq } => {
                self.db.execute(
                    "UPDATE messages SET delivered=1 WHERE author=0 AND id=? AND seq=?",
                    params![id, number(seq)?],
                )?;
                None
            }
            Content::Deleted { id, limit } => {
                let tx = self.db.transaction()?;
                let matches: i64 = tx.query_row(
                    "SELECT count(*) FROM deletions WHERE id=? AND lim=?",
                    params![id, number(limit)?],
                    |r| r.get(0),
                )?;
                if matches == 1 {
                    tx.execute(
                        "UPDATE deletions SET acked=1 WHERE lim<=? AND rowid<=(SELECT rowid FROM deletions WHERE id=? AND lim=?)",
                        params![number(limit)?,id,number(limit)?],
                    )?;
                }
                tx.commit()?;
                None
            }
            Content::Poll => None,
        };
        reply.map(|c| self.packet(c)).transpose()
    }
    pub fn view(&self) -> Result<View> {
        let mut stmt = self
            .db
            .prepare("SELECT author,body,delivered FROM messages ORDER BY rowid")?;
        let messages = stmt
            .query_map([], |r| {
                let b: Vec<u8> = r.get(1)?;
                Ok(Message {
                    mine: r.get::<_, i64>(0)? == 0,
                    text: String::from_utf8_lossy(&Zeroizing::new(b)).into_owned(),
                    delivered: r.get::<_, i64>(2)? == 1,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let deletion = self
            .db
            .query_row(
                "SELECT acked FROM deletions ORDER BY rowid DESC LIMIT 1",
                [],
                |r| r.get::<_, bool>(0),
            )
            .optional()?;
        let own_code = self.get("onion")?.map(|s| {
            Contact {
                onion: String::from_utf8_lossy(&s).into_owned(),
                pin: self.identity.pin(),
            }
            .code()
        });
        Ok(View {
            contact: self.contact()?.is_some(),
            own_code,
            messages,
            deletion,
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
