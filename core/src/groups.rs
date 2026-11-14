//! Fixed rosters, direct author-to-recipient delivery, and recipient-bound receipts.
use crate::{
    contact::Contact,
    vault::{
        Chat, Content, Conversation, Message, Packet, Peer, Vault, conversation_id, display_name,
    },
};
use anyhow::{Context, Result, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use rand::{RngCore, rngs::OsRng};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum GroupEvent {
    Join,
    JoinAck,
    Ready,
    ReadyAck,
    Wait,
    Text { id: String, seq: u64, body: String },
    Delivered { id: String, seq: u64 },
    Delete { id: String, limit: u64 },
    Deleted { id: String, limit: u64 },
}
impl GroupEvent {
    pub fn is_request(&self) -> bool {
        matches!(
            self,
            Self::Join | Self::Ready | Self::Text { .. } | Self::Delete { .. }
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roster {
    pub version: u8,
    pub nonce: String,
    pub name: String,
    pub members: Vec<Contact>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub id: String,
    pub roster: Roster,
}
fn random_id() -> String {
    let mut b = [0; 32];
    OsRng.fill_bytes(&mut b);
    hex::encode(b)
}
fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn integer(n: u64) -> Result<i64> {
    ensure!(n <= i64::MAX as u64, "invalid sequence");
    Ok(n as i64)
}
fn name_valid(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= 64 && !name.chars().any(char::is_control)
}
impl Invitation {
    fn digest(roster: &Roster) -> Result<String> {
        let mut hash = Sha256::new();
        hash.update(b"Psychotalk fixed group v1\0");
        hash.update(serde_json::to_vec(roster)?);
        Ok(hex::encode(hash.finalize()))
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.roster.version == 1
                && valid_id(&self.roster.nonce)
                && name_valid(&self.roster.name),
            "invalid group invitation"
        );
        ensure!(
            (3..=5).contains(&self.roster.members.len()),
            "group requires 3 to 5 identities"
        );
        for member in &self.roster.members {
            Contact::parse(&member.code())?;
        }
        ensure!(
            self.roster.members.windows(2).all(|m| m[0].pin < m[1].pin),
            "duplicate or noncanonical roster"
        );
        let onions: std::collections::HashSet<_> =
            self.roster.members.iter().map(|m| &m.onion).collect();
        ensure!(
            onions.len() == self.roster.members.len(),
            "duplicate endpoint"
        );
        ensure!(
            self.id == Self::digest(&self.roster)?,
            "group fingerprint mismatch"
        );
        Ok(())
    }
    pub fn code(&self) -> Result<String> {
        Ok(format!(
            "psychotalk-group:1:{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?)
        ))
    }
    pub fn parse(code: &str) -> Result<Self> {
        ensure!(code.len() <= 8192, "invitation too large");
        let bytes = URL_SAFE_NO_PAD.decode(
            code.trim()
                .strip_prefix("psychotalk-group:1:")
                .context("unsupported group invitation")?,
        )?;
        let invitation: Self = serde_json::from_slice(&bytes)?;
        invitation.validate()?;
        Ok(invitation)
    }
}
const SCHEMA:&str="
 CREATE TABLE groups(id TEXT PRIMARY KEY,name TEXT NOT NULL,invitation BLOB NOT NULL,activity INTEGER NOT NULL DEFAULT 0);
 CREATE TABLE group_members(gid TEXT NOT NULL REFERENCES groups(id),pin BLOB NOT NULL CHECK(length(pin)=32),onion TEXT NOT NULL,joined INTEGER NOT NULL,join_ack INTEGER NOT NULL,ready INTEGER NOT NULL,ready_ack INTEGER NOT NULL,PRIMARY KEY(gid,pin));
 CREATE TABLE group_counters(gid TEXT NOT NULL REFERENCES groups(id),author BLOB NOT NULL,seq INTEGER NOT NULL,watermark INTEGER NOT NULL,PRIMARY KEY(gid,author));
 CREATE TABLE group_messages(gid TEXT NOT NULL,author BLOB NOT NULL,id TEXT NOT NULL,seq INTEGER NOT NULL,body BLOB NOT NULL,PRIMARY KEY(gid,author,id),UNIQUE(gid,author,seq),FOREIGN KEY(gid,author) REFERENCES group_counters(gid,author));
 CREATE TABLE group_deliveries(gid TEXT NOT NULL,author BLOB NOT NULL,id TEXT NOT NULL,recipient BLOB NOT NULL,acked INTEGER NOT NULL,PRIMARY KEY(gid,author,id,recipient),FOREIGN KEY(gid,author,id) REFERENCES group_messages(gid,author,id) ON DELETE CASCADE);
 CREATE TABLE group_deletions(gid TEXT NOT NULL,author BLOB NOT NULL,id TEXT NOT NULL,lim INTEGER NOT NULL,PRIMARY KEY(gid,author,id),FOREIGN KEY(gid,author) REFERENCES group_counters(gid,author));
 CREATE TABLE group_delete_acks(gid TEXT NOT NULL,author BLOB NOT NULL,id TEXT NOT NULL,recipient BLOB NOT NULL,acked INTEGER NOT NULL,PRIMARY KEY(gid,author,id,recipient),FOREIGN KEY(gid,author,id) REFERENCES group_deletions(gid,author,id));";
pub(crate) fn migrate(db: &mut Connection) -> Result<()> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(version == 2 || version == 3, "unsupported groups schema");
    if version == 3 {
        return Ok(());
    }
    let tx = db.transaction()?;
    tx.execute_batch(SCHEMA)?;
    tx.pragma_update(None, "user_version", 3)?;
    tx.commit()?;
    Ok(())
}
fn touch(db: &Connection, gid: &str) -> Result<()> {
    db.execute("UPDATE groups SET activity=1+max((SELECT COALESCE(MAX(activity),0) FROM groups),(SELECT COALESCE(MAX(activity),0) FROM conversations)) WHERE id=?",[gid])?;
    Ok(())
}
fn ready_self(db: &Connection, gid: &str, own: [u8; 32]) -> Result<()> {
    db.execute("UPDATE group_members SET ready=1,ready_ack=1 WHERE gid=? AND pin=? AND NOT EXISTS(SELECT 1 FROM group_members WHERE gid=? AND joined=0)",params![gid,own.as_slice(),gid])?;
    Ok(())
}
fn packet(own: [u8; 32], gid: &str, event: GroupEvent) -> Result<Packet> {
    let id: [u8; 32] = hex::decode(gid)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("invalid group id"))?;
    Ok(Packet {
        version: 2,
        conversation: id,
        author: own,
        content: Content::Group { event },
    })
}
impl Vault {
    pub fn is_group(&self, gid: &str) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM groups WHERE id=?)",
            [gid],
            |r| r.get(0),
        )?)
    }
    pub fn group_invitation(&self, gid: &str) -> Result<Invitation> {
        let bytes: Vec<u8> =
            self.db
                .query_row("SELECT invitation FROM groups WHERE id=?", [gid], |r| {
                    r.get(0)
                })?;
        Ok(serde_json::from_slice(&bytes)?)
    }
    pub fn preview_group(&self, code: &str) -> Result<Value> {
        let inv = Invitation::parse(code)?;
        ensure!(
            inv.roster
                .members
                .iter()
                .any(|p| p.pin == self.identity.pin()),
            "this identity is not invited"
        );
        Ok(
            json!({"id":inv.id,"name":inv.roster.name,"members":inv.roster.members.iter().map(|p|json!({"fingerprint":hex::encode(p.pin),"code":p.code()})).collect::<Vec<_>>()}),
        )
    }
    pub fn create_group(&mut self, name: &str, cids: &[String]) -> Result<String> {
        ensure!(
            (2..=4).contains(&cids.len()) && name_valid(name.trim()),
            "invalid group size or name"
        );
        let own = Contact {
            onion: String::from_utf8(self.get("onion")?.context("onion not initialized")?)?,
            pin: self.identity.pin(),
        };
        let mut members = vec![own];
        for cid in cids {
            let peer = self.peer(cid)?;
            members.push(Contact {
                onion: peer.onion,
                pin: peer.pin,
            });
        }
        members.sort_by_key(|p| p.pin);
        let roster = Roster {
            version: 1,
            nonce: random_id(),
            name: name.trim().into(),
            members,
        };
        let inv = Invitation {
            id: Invitation::digest(&roster)?,
            roster,
        };
        inv.validate()?;
        self.import_group(&inv.code()?)
    }
    pub fn import_group(&mut self, code: &str) -> Result<String> {
        let inv = Invitation::parse(code)?;
        let own = self.identity.pin();
        let me = inv
            .roster
            .members
            .iter()
            .find(|p| p.pin == own)
            .context("this identity is not invited")?;
        ensure!(
            self.get("onion")?.as_deref() == Some(me.onion.as_bytes()),
            "invited endpoint does not match this installation"
        );
        let bytes = serde_json::to_vec(&inv)?;
        let tx = self.db.transaction()?;
        let previous: Option<Vec<u8>> = tx
            .query_row("SELECT invitation FROM groups WHERE id=?", [&inv.id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(previous) = previous {
            ensure!(previous == bytes, "roster cannot change");
            return Ok(inv.id);
        }
        tx.execute(
            "INSERT INTO groups(id,name,invitation) VALUES(?,?,?)",
            params![inv.id, inv.roster.name, bytes],
        )?;
        for member in &inv.roster.members {
            if member.pin != own {
                tx.execute("INSERT INTO contacts(pin,onion,tagname) VALUES(?,?,'') ON CONFLICT(pin) DO NOTHING",params![member.pin.as_slice(),member.onion])?;
                let onion: String = tx.query_row(
                    "SELECT onion FROM contacts WHERE pin=?",
                    [member.pin.as_slice()],
                    |r| r.get(0),
                )?;
                ensure!(
                    onion == member.onion,
                    "endpoint conflict; existing pin cannot be replaced"
                );
            }
            let self_member = member.pin == own;
            tx.execute(
                "INSERT INTO group_members VALUES(?,?,?,?,?,0,0)",
                params![
                    inv.id,
                    member.pin.as_slice(),
                    member.onion,
                    self_member,
                    self_member
                ],
            )?;
            tx.execute(
                "INSERT INTO group_counters VALUES(?,?,0,0)",
                params![inv.id, member.pin.as_slice()],
            )?;
        }
        touch(&tx, &inv.id)?;
        tx.commit()?;
        Ok(inv.id)
    }
    pub fn group_peers(&self) -> Result<Vec<Peer>> {
        let mut stmt=self.db.prepare("SELECT DISTINCT m.pin,m.onion,c.tagname FROM group_members m JOIN contacts c ON c.pin=m.pin WHERE m.pin<>? ORDER BY m.pin")?;
        let own = self.identity.pin();
        Ok(stmt
            .query_map([own.as_slice()], |r| {
                let p: Vec<u8> = r.get(0)?;
                let pin: [u8; 32] = p.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(Peer {
                    id: hex::encode(conversation_id(own, pin)),
                    pin,
                    onion: r.get(1)?,
                    tagname: display_name(r.get(2)?, &pin),
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn group_ids_for_peer(&self, pin: [u8; 32]) -> Result<Vec<String>> {
        let mut s = self
            .db
            .prepare("SELECT gid FROM group_members WHERE pin=? ORDER BY gid")?;
        Ok(s.query_map([pin.as_slice()], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
    fn group_member(&self, gid: &str, pin: [u8; 32]) -> Result<()> {
        ensure!(
            self.db.query_row(
                "SELECT EXISTS(SELECT 1 FROM group_members WHERE gid=? AND pin=?)",
                params![gid, pin.as_slice()],
                |r| r.get::<_, bool>(0)
            )?,
            "identity is not a group member"
        );
        Ok(())
    }
    pub fn group_send(&mut self, gid: &str, text: &str) -> Result<()> {
        ensure!(!text.is_empty() && text.len() <= 4096, "invalid group text");
        let own = self.identity.pin();
        self.group_member(gid, own)?;
        ensure!(
            self.db.query_row(
                "SELECT count(*) FROM group_members WHERE gid=? AND (joined=0 OR ready=0)",
                [gid],
                |r| r.get::<_, i64>(0)
            )? == 0,
            "group is awaiting member acceptance/synchronization"
        );
        let tx = self.db.transaction()?;
        let seq: i64 = tx.query_row(
            "SELECT seq FROM group_counters WHERE gid=? AND author=?",
            params![gid, own.as_slice()],
            |r| r.get(0),
        )?;
        let seq = seq.checked_add(1).context("sequence exhausted")?;
        tx.execute(
            "UPDATE group_counters SET seq=? WHERE gid=? AND author=?",
            params![seq, gid, own.as_slice()],
        )?;
        let id = random_id();
        tx.execute(
            "INSERT INTO group_messages VALUES(?,?,?,?,?)",
            params![gid, own.as_slice(), id, seq, text.as_bytes()],
        )?;
        tx.execute("INSERT INTO group_deliveries SELECT ?,?,?,pin,0 FROM group_members WHERE gid=? AND pin<>?",params![gid,own.as_slice(),id,gid,own.as_slice()])?;
        touch(&tx, gid)?;
        tx.commit()?;
        Ok(())
    }
    pub fn group_delete_sent(&mut self, gid: &str) -> Result<()> {
        let own = self.identity.pin();
        self.group_member(gid, own)?;
        let tx = self.db.transaction()?;
        let limit: i64 = tx.query_row(
            "SELECT seq FROM group_counters WHERE gid=? AND author=?",
            params![gid, own.as_slice()],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE group_counters SET watermark=max(watermark,?) WHERE gid=? AND author=?",
            params![limit, gid, own.as_slice()],
        )?;
        tx.execute(
            "DELETE FROM group_messages WHERE gid=? AND author=? AND seq<=?",
            params![gid, own.as_slice(), limit],
        )?;
        let id = random_id();
        tx.execute(
            "INSERT INTO group_deletions VALUES(?,?,?,?)",
            params![gid, own.as_slice(), id, limit],
        )?;
        tx.execute("INSERT INTO group_delete_acks SELECT ?,?,?,pin,0 FROM group_members WHERE gid=? AND pin<>?",params![gid,own.as_slice(),id,gid,own.as_slice()])?;
        touch(&tx, gid)?;
        tx.commit()?;
        Ok(())
    }
    pub fn group_next(&self, gid: &str, recipient: [u8; 32]) -> Result<Option<Packet>> {
        self.group_member(gid, recipient)?;
        let own = self.identity.pin();
        ensure!(recipient != own, "self delivery forbidden");
        let deletion:Option<(String,i64)>=self.db.query_row("SELECT d.id,d.lim FROM group_deletions d JOIN group_delete_acks a ON a.gid=d.gid AND a.author=d.author AND a.id=d.id WHERE d.gid=? AND d.author=? AND a.recipient=? AND a.acked=0 ORDER BY d.lim DESC,d.rowid DESC LIMIT 1",params![gid,own.as_slice(),recipient.as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((id, limit)) = deletion {
            return Ok(Some(packet(
                own,
                gid,
                GroupEvent::Delete {
                    id,
                    limit: limit as u64,
                },
            )?));
        }
        let (join_ack, peer_ready, ready_ack): (bool, bool, bool) = self.db.query_row(
            "SELECT join_ack,ready,ready_ack FROM group_members WHERE gid=? AND pin=?",
            params![gid, recipient.as_slice()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if !join_ack {
            return Ok(Some(packet(own, gid, GroupEvent::Join)?));
        }
        let own_ready: bool = self.db.query_row(
            "SELECT ready FROM group_members WHERE gid=? AND pin=?",
            params![gid, own.as_slice()],
            |r| r.get(0),
        )?;
        if own_ready && !ready_ack {
            return Ok(Some(packet(own, gid, GroupEvent::Ready)?));
        }
        if own_ready && peer_ready {
            let text:Option<(String,i64,Vec<u8>)>=self.db.query_row("SELECT m.id,m.seq,m.body FROM group_messages m JOIN group_deliveries d ON d.gid=m.gid AND d.author=m.author AND d.id=m.id WHERE m.gid=? AND m.author=? AND d.recipient=? AND d.acked=0 AND m.seq>(SELECT watermark FROM group_counters WHERE gid=? AND author=?) ORDER BY m.seq LIMIT 1",params![gid,own.as_slice(),recipient.as_slice(),gid,own.as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            if let Some((id, seq, bytes)) = text {
                return Ok(Some(packet(
                    own,
                    gid,
                    GroupEvent::Text {
                        id,
                        seq: seq as u64,
                        body: STANDARD.encode(Zeroizing::new(bytes)),
                    },
                )?));
            }
        }
        Ok(None)
    }
    pub fn apply_group(
        &mut self,
        authenticated: [u8; 32],
        mut p: Packet,
    ) -> Result<Option<Packet>> {
        ensure!(
            p.version == 2 && p.author == authenticated,
            "invalid group authentication"
        );
        let gid = hex::encode(p.conversation);
        let own = self.identity.pin();
        let Content::Group { event } = std::mem::replace(&mut p.content, Content::Poll) else {
            anyhow::bail!("invalid group envelope");
        };
        if !self.is_group(&gid)? {
            return if event.is_request() {
                Ok(Some(packet(own, &gid, GroupEvent::Wait)?))
            } else {
                Ok(None)
            };
        }
        self.group_member(&gid, authenticated)?;
        ensure!(authenticated != own, "invalid self author");
        let event = match event {
            GroupEvent::Join => {
                let tx = self.db.transaction()?;
                tx.execute(
                    "UPDATE group_members SET joined=1 WHERE gid=? AND pin=?",
                    params![gid, authenticated.as_slice()],
                )?;
                ready_self(&tx, &gid, own)?;
                tx.commit()?;
                Some(GroupEvent::JoinAck)
            }
            GroupEvent::JoinAck => {
                self.db.execute(
                    "UPDATE group_members SET join_ack=1 WHERE gid=? AND pin=?",
                    params![gid, authenticated.as_slice()],
                )?;
                None
            }
            GroupEvent::Ready => {
                let joined: bool = self.db.query_row(
                    "SELECT joined FROM group_members WHERE gid=? AND pin=?",
                    params![gid, authenticated.as_slice()],
                    |r| r.get(0),
                )?;
                ensure!(joined, "member has not accepted roster");
                self.db.execute(
                    "UPDATE group_members SET ready=1 WHERE gid=? AND pin=?",
                    params![gid, authenticated.as_slice()],
                )?;
                Some(GroupEvent::ReadyAck)
            }
            GroupEvent::ReadyAck => {
                let ready: bool = self.db.query_row(
                    "SELECT ready FROM group_members WHERE gid=? AND pin=?",
                    params![gid, own.as_slice()],
                    |r| r.get(0),
                )?;
                ensure!(ready, "premature ready acknowledgement");
                self.db.execute(
                    "UPDATE group_members SET ready_ack=1 WHERE gid=? AND pin=?",
                    params![gid, authenticated.as_slice()],
                )?;
                None
            }
            GroupEvent::Wait => None,
            GroupEvent::Text { id, seq, body } => {
                ensure!(
                    valid_id(&id) && seq > 0 && body.len() <= 5464,
                    "invalid group message"
                );
                let seq = integer(seq)?;
                let encoded = Zeroizing::new(body);
                let bytes = Zeroizing::new(STANDARD.decode(encoded.as_bytes())?);
                ensure!(
                    !bytes.is_empty() && bytes.len() <= 4096 && std::str::from_utf8(&bytes).is_ok(),
                    "invalid group text"
                );
                let ready: bool = self.db.query_row(
                    "SELECT ready FROM group_members WHERE gid=? AND pin=?",
                    params![gid, own.as_slice()],
                    |r| r.get(0),
                )?;
                if !ready {
                    return Ok(Some(packet(own, &gid, GroupEvent::Wait)?));
                }
                let tx = self.db.transaction()?;
                let limit: i64 = tx.query_row(
                    "SELECT watermark FROM group_counters WHERE gid=? AND author=?",
                    params![gid, authenticated.as_slice()],
                    |r| r.get(0),
                )?;
                if seq > limit {
                    let existing: Option<(String, Vec<u8>)> = tx
                        .query_row(
                            "SELECT id,body FROM group_messages WHERE gid=? AND author=? AND seq=?",
                            params![gid, authenticated.as_slice(), seq],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    if let Some((old, oldbytes)) = existing {
                        ensure!(
                            old == id && Zeroizing::new(oldbytes).as_slice() == bytes.as_slice(),
                            "conflicting group retransmission"
                        );
                    } else {
                        tx.execute(
                            "INSERT INTO group_messages VALUES(?,?,?,?,?)",
                            params![gid, authenticated.as_slice(), id, seq, bytes.as_slice()],
                        )?;
                        touch(&tx, &gid)?;
                    }
                }
                tx.commit()?;
                Some(GroupEvent::Delivered {
                    id,
                    seq: seq as u64,
                })
            }
            GroupEvent::Delivered { id, seq } => {
                self.db.execute("UPDATE group_deliveries SET acked=1 WHERE gid=? AND author=? AND recipient=? AND id=? AND EXISTS(SELECT 1 FROM group_messages m WHERE m.gid=group_deliveries.gid AND m.author=group_deliveries.author AND m.id=group_deliveries.id AND m.seq=?)",params![gid,own.as_slice(),authenticated.as_slice(),id,integer(seq)?])?;
                None
            }
            GroupEvent::Delete { id, limit } => {
                ensure!(valid_id(&id), "invalid deletion operation");
                let limit = integer(limit)?;
                let tx = self.db.transaction()?;
                tx.execute(
                    "UPDATE group_counters SET watermark=max(watermark,?) WHERE gid=? AND author=?",
                    params![limit, gid, authenticated.as_slice()],
                )?;
                tx.execute("DELETE FROM group_messages WHERE gid=? AND author=? AND seq<=(SELECT watermark FROM group_counters WHERE gid=? AND author=?)",params![gid,authenticated.as_slice(),gid,authenticated.as_slice()])?;
                tx.commit()?;
                Some(GroupEvent::Deleted {
                    id,
                    limit: limit as u64,
                })
            }
            GroupEvent::Deleted { id, limit } => {
                self.db.execute("UPDATE group_delete_acks SET acked=1 WHERE gid=? AND author=? AND recipient=? AND id IN(SELECT id FROM group_deletions WHERE gid=? AND author=? AND lim<=? AND rowid<=(SELECT rowid FROM group_deletions WHERE gid=? AND author=? AND id=? AND lim=?))",params![gid,own.as_slice(),authenticated.as_slice(),gid,own.as_slice(),integer(limit)?,gid,own.as_slice(),id,integer(limit)?])?;
                None
            }
        };
        event.map(|event| packet(own, &gid, event)).transpose()
    }
    pub fn group_rename_member(&self, gid: &str, pin: [u8; 32], name: &str) -> Result<()> {
        self.group_member(gid, pin)?;
        ensure!(pin != self.identity.pin(), "own label is reserved");
        let name = name.trim();
        ensure!(
            name.chars().count() <= 64 && !name.chars().any(char::is_control),
            "invalid tagname"
        );
        self.db.execute(
            "UPDATE contacts SET tagname=? WHERE pin=?",
            params![name, pin.as_slice()],
        )?;
        Ok(())
    }
    pub fn group_chat(&self, gid: &str) -> Result<Chat> {
        let inv = self.group_invitation(gid)?;
        let own = self.identity.pin();
        let total = inv.roster.members.len() - 1;
        let mut stmt=self.db.prepare("SELECT m.author,m.body,COALESCE(c.tagname,''),(SELECT count(*) FROM group_deliveries d WHERE d.gid=m.gid AND d.author=m.author AND d.id=m.id AND d.acked=1) FROM group_messages m LEFT JOIN contacts c ON c.pin=m.author WHERE m.gid=? ORDER BY m.rowid")?;
        let messages = stmt
            .query_map([gid], |r| {
                let author: Vec<u8> = r.get(0)?;
                let bytes: Vec<u8> = r.get(1)?;
                let mine = author == own;
                let count = r.get::<_, i64>(3)? as usize;
                Ok(Message {
                    mine,
                    text: String::from_utf8_lossy(&Zeroizing::new(bytes)).into_owned(),
                    delivered: !mine || count == total,
                    author: if mine {
                        "Você".into()
                    } else {
                        display_name(r.get(2)?, &author)
                    },
                    delivered_count: count,
                    recipient_count: if mine { total } else { 0 },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt=self.db.prepare("SELECT m.pin,m.onion,COALESCE(c.tagname,''),m.joined,m.ready FROM group_members m LEFT JOIN contacts c ON c.pin=m.pin WHERE m.gid=? ORDER BY m.pin")?;
        let members:Vec<Value>=stmt.query_map([gid],|r|{let pin:Vec<u8>=r.get(0)?;Ok(json!({"fingerprint":hex::encode(&pin),"name":if pin==own{"Você".into()}else{display_name(r.get(2)?,&pin)},"joined":r.get::<_,bool>(3)?,"ready":r.get::<_,bool>(4)?,"code":Contact{pin:pin.try_into().map_err(|_|rusqlite::Error::InvalidQuery)?,onion:r.get(1)?}.code()}))})?.collect::<rusqlite::Result<_>>()?;
        let latest:Option<String>=self.db.query_row("SELECT id FROM group_deletions WHERE gid=? AND author=? ORDER BY rowid DESC LIMIT 1",params![gid,own.as_slice()],|r|r.get(0)).optional()?;
        let progress = if let Some(id) = latest {
            let mut q=self.db.prepare("SELECT a.recipient,COALESCE(c.tagname,''),a.acked FROM group_delete_acks a LEFT JOIN contacts c ON c.pin=a.recipient WHERE a.gid=? AND a.author=? AND a.id=? ORDER BY a.recipient")?;
            let recipients:Vec<Value>=q.query_map(params![gid,own.as_slice(),id],|r|{let pin:Vec<u8>=r.get(0)?;Ok(json!({"name":display_name(r.get(1)?,&pin),"fingerprint":hex::encode(pin),"confirmed":r.get::<_,bool>(2)?}))})?.collect::<rusqlite::Result<_>>()?;
            Some(
                json!({"confirmed":recipients.iter().filter(|p|p["confirmed"]==true).count(),"total":total,"recipients":recipients}),
            )
        } else {
            None
        };
        let joined = members.iter().filter(|m| m["joined"] == true).count();
        let ready = members.iter().filter(|m| m["ready"] == true).count();
        Ok(Chat {
            id: gid.into(),
            tagname: inv.roster.name.clone(),
            messages,
            deletion: progress.as_ref().map(|p| p["confirmed"] == p["total"]),
            kind: "group".into(),
            group: Some(
                json!({"members":members,"joined":joined,"ready":ready,"total":total+1,"invitation":inv.code()?,"fingerprint":gid,"deletion":progress}),
            ),
        })
    }
    pub fn group_summaries(&self) -> Result<Vec<(Conversation, i64)>> {
        let own = self.identity.pin();
        let mut stmt=self.db.prepare("SELECT g.id,g.name,g.activity,(SELECT count(*) FROM group_deliveries d WHERE d.gid=g.id AND d.author=? AND d.acked=0),EXISTS(SELECT 1 FROM group_delete_acks a WHERE a.gid=g.id AND a.author=? AND a.acked=0) FROM groups g ORDER BY g.id")?;
        Ok(stmt
            .query_map(params![own.as_slice(), own.as_slice()], |r| {
                let id: String = r.get(0)?;
                Ok((
                    Conversation {
                        id: id.clone(),
                        tagname: r.get(1)?,
                        fingerprint: id,
                        pending: r.get(3)?,
                        deletion_pending: r.get(4)?,
                        kind: "group".into(),
                    },
                    r.get(2)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}
