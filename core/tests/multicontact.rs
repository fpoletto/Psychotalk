use psychotalk_core::{
    contact::Contact,
    tls::Identity,
    vault::{Content, Packet, Vault, conversation_id},
};
use rusqlite::{Connection, params};
use sha3::{Digest, Sha3_256};
use tempfile::TempDir;
fn onion(seed: u8) -> String {
    let key = [seed; 32];
    let mut h = Sha3_256::new();
    h.update(b".onion checksum");
    h.update(key);
    h.update([3]);
    let mut raw = key.to_vec();
    raw.extend_from_slice(&h.finalize()[..2]);
    raw.push(3);
    format!(
        "{}.onion",
        data_encoding::BASE32_NOPAD
            .encode(&raw)
            .to_ascii_lowercase()
    )
}
fn code(v: &Vault, n: u8) -> String {
    Contact {
        onion: onion(n),
        pin: v.identity.pin(),
    }
    .code()
}
fn create(root: &TempDir, name: &str, key: u8) -> Vault {
    Vault::open(&root.path().join(name), &[key; 32], true).unwrap()
}
fn deliver(a: &mut Vault, b: &mut Vault, cid: &str) {
    let pin = a.identity.pin();
    let ack = b
        .apply_from(pin, a.next_for(cid).unwrap())
        .unwrap()
        .unwrap();
    a.apply_from(b.identity.pin(), ack).unwrap();
}
#[test]
fn isolated_histories_counters_receipts_and_authorship() {
    let root = TempDir::new().unwrap();
    let mut a = create(&root, "a", 1);
    let mut b = create(&root, "b", 2);
    let mut c = create(&root, "c", 3);
    let ab = a.pair(&code(&b, 2)).unwrap();
    let ac = a.pair(&code(&c, 3)).unwrap();
    let ba = b.pair(&code(&a, 1)).unwrap();
    let ca = c.pair(&code(&a, 1)).unwrap();
    a.send_to(&ab, "for b").unwrap();
    a.send_to(&ac, "for c").unwrap();
    assert!(matches!(
        a.next_for(&ab).unwrap().content,
        Content::Message { seq: 1, .. }
    ));
    assert!(matches!(
        a.next_for(&ac).unwrap().content,
        Content::Message { seq: 1, .. }
    ));
    let original = a.next_for(&ab).unwrap();
    let bytes = serde_json::to_vec(&original).unwrap();
    let wrong = c
        .packet_for(
            &ca,
            Content::Delivered {
                id: match &original.content {
                    Content::Message { id, .. } => id.clone(),
                    _ => panic!(),
                },
                seq: 1,
            },
        )
        .unwrap();
    a.apply_from(c.identity.pin(), wrong).unwrap();
    assert!(!a.chat(&ab).unwrap().messages[0].delivered);
    deliver(&mut a, &mut b, &ab);
    deliver(&mut a, &mut c, &ac);
    assert_eq!(b.chat(&ba).unwrap().messages[0].text, "for b");
    assert_eq!(c.chat(&ca).unwrap().messages[0].text, "for c");
    let mut forged: Packet = serde_json::from_slice(&bytes).unwrap();
    forged.conversation = conversation_id(a.identity.pin(), c.identity.pin());
    assert!(b.apply_from(a.identity.pin(), forged).is_err());
    let mut forged: Packet = serde_json::from_slice(&bytes).unwrap();
    forged.author = c.identity.pin();
    assert!(b.apply_from(a.identity.pin(), forged).is_err());
    assert!(
        b.apply_from(c.identity.pin(), c.packet_for(&ca, Content::Poll).unwrap())
            .is_err()
    );
    a.delete_sent(&ab).unwrap();
    a.delete_sent(&ac).unwrap();
    let ack = b
        .apply_from(a.identity.pin(), a.next_for(&ab).unwrap())
        .unwrap()
        .unwrap();
    a.apply_from(b.identity.pin(), ack).unwrap();
    assert_eq!(a.chat(&ab).unwrap().deletion, Some(true));
    assert_eq!(a.chat(&ac).unwrap().deletion, Some(false));
    assert!(b.chat(&ba).unwrap().messages.is_empty());
    assert_eq!(c.chat(&ca).unwrap().messages.len(), 1);
    a.send_to(&ab, "new b").unwrap();
    assert!(matches!(
        a.next_for(&ab).unwrap().content,
        Content::Message { seq: 2, .. }
    ));
    assert!(a.send("ambiguous").is_err());
    assert!(a.delete_all().is_err());
}
#[test]
fn tagnames_are_local_persistent_and_do_not_change_credentials() {
    let root = TempDir::new().unwrap();
    let mut a = create(&root, "a", 1);
    let b = create(&root, "b", 2);
    let c = create(&root, "c", 3);
    let ab = a.pair_named(&code(&b, 2), "@Bia").unwrap();
    let ac = a.pair_named(&code(&c, 3), "@Bia").unwrap();
    let own = a.identity.pin();
    a.rename(&ab, "  Amiga 😀  ").unwrap();
    assert_eq!(a.peer(&ab).unwrap().tagname, "Amiga 😀");
    assert_eq!(a.peer(&ac).unwrap().tagname, "@Bia");
    assert_eq!(a.peer(&ab).unwrap().pin, b.identity.pin());
    assert!(a.rename(&ab, &"x".repeat(65)).is_err());
    assert!(a.rename(&ab, "bad\nname").is_err());
    assert!(a.pair(&code(&b, 9)).is_err());
    assert!(
        a.pair(
            &Contact {
                onion: onion(2),
                pin: c.identity.pin()
            }
            .code()
        )
        .is_err()
    );
    let packet = serde_json::to_string(&a.next_for(&ab).unwrap()).unwrap();
    assert!(!packet.contains("Amiga"));
    assert!(!packet.contains("@Bia"));
    drop(a);
    let a = Vault::open(&root.path().join("a"), &[1; 32], false).unwrap();
    assert_eq!(a.identity.pin(), own);
    assert_eq!(a.peer(&ab).unwrap().tagname, "Amiga 😀");
    a.rename(&ab, "").unwrap();
    assert!(a.peer(&ab).unwrap().tagname.starts_with("Contato "));
}
fn legacy(root: &TempDir, bad: bool) -> ([u8; 32], [u8; 32]) {
    let identity = Identity::generate().unwrap();
    let peer = Identity::generate().unwrap();
    let db = Connection::open(root.path().join("legacy")).unwrap();
    db.execute_batch(&format!("PRAGMA key=\"x'{}'\"; CREATE TABLE config(k TEXT PRIMARY KEY,v BLOB NOT NULL); CREATE TABLE counters(author INTEGER PRIMARY KEY,seq INTEGER NOT NULL,watermark INTEGER NOT NULL); INSERT INTO counters VALUES(0,9,4),(1,0,3); CREATE TABLE messages(id TEXT PRIMARY KEY,author INTEGER NOT NULL,seq INTEGER NOT NULL,body BLOB NOT NULL,delivered INTEGER NOT NULL,UNIQUE(author,seq)); CREATE TABLE deletions(id TEXT PRIMARY KEY,lim INTEGER NOT NULL,acked INTEGER NOT NULL);",hex::encode([7;32]))).unwrap();
    let contact = Contact {
        onion: if bad { "invalid".into() } else { onion(2) },
        pin: peer.pin(),
    };
    for (k, v) in [
        ("identity", serde_json::to_vec(&identity).unwrap()),
        ("contact", serde_json::to_vec(&contact).unwrap()),
        ("onion", onion(1).into_bytes()),
        ("onion-key", b"opaque-test-key".to_vec()),
    ] {
        db.execute("INSERT INTO config VALUES(?,?)", params![k, v])
            .unwrap();
    }
    db.execute(
        "INSERT INTO messages VALUES(?,0,9,?,0)",
        params!["1".repeat(32), b"queued".as_slice()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO messages VALUES(?,1,8,?,1)",
        params!["2".repeat(32), b"keep peer".as_slice()],
    )
    .unwrap();
    db.execute("INSERT INTO deletions VALUES(?,4,0)", ["3".repeat(32)])
        .unwrap();
    (identity.pin(), peer.pin())
}
#[test]
fn migrates_real_legacy_sqlcipher_without_losing_pending_work_or_identity() {
    let root = TempDir::new().unwrap();
    let (own, peer) = legacy(&root, false);
    let mut v = Vault::open(&root.path().join("legacy"), &[7; 32], false).unwrap();
    let cid = v.only_conversation().unwrap();
    assert_eq!(v.identity.pin(), own);
    assert_eq!(v.peer(&cid).unwrap().pin, peer);
    assert_eq!(v.get("onion-key").unwrap().unwrap(), b"opaque-test-key");
    assert_eq!(v.chat(&cid).unwrap().messages.len(), 2);
    assert_eq!(v.chat(&cid).unwrap().deletion, Some(false));
    let pending = v.next_for(&cid).unwrap();
    assert!(matches!(pending.content, Content::Delete { limit: 4, .. }));
    let id = match &pending.content {
        Content::Delete { id, .. } => id.clone(),
        _ => panic!(),
    };
    v.apply_from(
        peer,
        Packet {
            version: 1,
            author: peer,
            conversation: conversation_id(own, peer),
            content: Content::Deleted { id, limit: 4 },
        },
    )
    .unwrap();
    assert!(matches!(
        v.next_for(&cid).unwrap().content,
        Content::Message { seq: 9, .. }
    ));
    v.send_to(&cid, "next").unwrap();
    let old = Packet {
        version: 1,
        author: peer,
        conversation: conversation_id(own, peer),
        content: Content::Message {
            id: "f".repeat(32),
            seq: 2,
            body: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"deleted"),
        },
    };
    v.apply_from(peer, old).unwrap();
    assert_eq!(v.chat(&cid).unwrap().messages.len(), 3);
    drop(v);
    let v = Vault::open(&root.path().join("legacy"), &[7; 32], false).unwrap();
    assert_eq!(v.identity.pin(), own);
    assert_eq!(v.chat(&cid).unwrap().messages.len(), 3);
}
#[test]
fn malformed_legacy_contact_rolls_back_schema_and_keeps_original_rows() {
    let root = TempDir::new().unwrap();
    legacy(&root, true);
    assert!(Vault::open(&root.path().join("legacy"), &[7; 32], false).is_err());
    let db = Connection::open(root.path().join("legacy")).unwrap();
    db.execute_batch(&format!("PRAGMA key=\"x'{}'\";", hex::encode([7; 32])))
        .unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('contacts','legacy_messages')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
