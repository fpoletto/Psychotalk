use psychotalk_core::{
    contact::Contact,
    groups::{GroupEvent, Invitation},
    vault::{Content, Packet, Vault},
};
use rusqlite::Connection;
use sha3::{Digest, Sha3_256};
use tempfile::TempDir;
fn onion(n: u8) -> String {
    let mut h = Sha3_256::new();
    h.update(b".onion checksum");
    h.update([n; 32]);
    h.update([3]);
    let mut raw = vec![n; 32];
    raw.extend_from_slice(&h.finalize()[..2]);
    raw.push(3);
    format!(
        "{}.onion",
        data_encoding::BASE32_NOPAD
            .encode(&raw)
            .to_ascii_lowercase()
    )
}
fn nodes(root: &TempDir, n: usize) -> Vec<Vault> {
    (0..n)
        .map(|i| {
            let v =
                Vault::open(&root.path().join(i.to_string()), &[i as u8 + 1; 32], true).unwrap();
            v.put("onion", onion(i as u8 + 1).as_bytes()).unwrap();
            v
        })
        .collect()
}
fn code(v: &Vault) -> String {
    Contact {
        pin: v.identity.pin(),
        onion: String::from_utf8(v.get("onion").unwrap().unwrap()).unwrap(),
    }
    .code()
}
fn create(v: &mut [Vault]) -> String {
    let mut cids = Vec::new();
    for i in 1..v.len() {
        cids.push(
            v[0].pair_named(&code(&v[i]), &format!("@member{i}"))
                .unwrap(),
        );
    }
    v[0].create_group("Equipe fixa", &cids).unwrap()
}
fn import(v: &mut [Vault], gid: &str) {
    let code = v[0].group_invitation(gid).unwrap().code().unwrap();
    for member in &mut v[1..] {
        assert_eq!(member.import_group(&code).unwrap(), gid);
    }
}
fn route(v: &mut [Vault], a: usize, b: usize, gid: &str) -> Option<Packet> {
    let p = v[a].group_next(gid, v[b].identity.pin()).unwrap()?;
    let pin = v[a].identity.pin();
    let ack = v[b].apply_group(pin, p).unwrap();
    if let Some(ref ack) = ack {
        let pin = v[b].identity.pin();
        v[a].apply_group(pin, ack.clone()).unwrap();
    }
    ack
}
fn sync(v: &mut [Vault], gid: &str) {
    for _ in 0..6 {
        for a in 0..v.len() {
            for b in 0..v.len() {
                if a != b {
                    route(v, a, b, gid);
                }
            }
        }
    }
}
fn progress(v: &Vault, gid: &str) -> serde_json::Value {
    v.group_chat(gid).unwrap().group.unwrap()["deletion"].clone()
}
#[test]
fn fixed_three_and_five_member_groups_require_every_acceptance_and_ready() {
    for n in [3, 5] {
        let root = TempDir::new().unwrap();
        let mut v = nodes(&root, n);
        let gid = create(&mut v);
        assert!(v[0].group_send(&gid, "too early").is_err());
        let invite = v[0].group_invitation(&gid).unwrap().code().unwrap();
        assert_eq!(
            v[1].preview_group(&invite).unwrap()["members"]
                .as_array()
                .unwrap()
                .len(),
            n
        );
        assert!(!v[1].is_group(&gid).unwrap());
        import(&mut v, &gid);
        assert!(v[0].group_send(&gid, "still too early").is_err());
        sync(&mut v, &gid);
        for member in &mut v {
            member.group_send(&gid, "every author").unwrap();
        }
        sync(&mut v, &gid);
        for member in &v {
            let chat = member.group_chat(&gid).unwrap();
            assert_eq!(chat.messages.len(), n);
            let own = chat.messages.iter().find(|m| m.mine).unwrap();
            assert!(own.delivered);
            assert_eq!(own.delivered_count, n - 1);
        }
    }
}
#[test]
fn receipts_and_deletion_are_per_recipient_and_only_cover_the_author() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 5);
    let gid = create(&mut v);
    import(&mut v, &gid);
    sync(&mut v, &gid);
    v[0].group_send(&gid, "author zero").unwrap();
    v[1].group_send(&gid, "preserve author one").unwrap();
    sync(&mut v, &gid);
    let late = v[0].group_next(&gid, v[4].identity.pin()).unwrap();
    assert!(late.is_none());
    v[0].group_send(&gid, "not yet everywhere").unwrap();
    route(&mut v, 0, 1, &gid);
    let msg = v[0].group_chat(&gid).unwrap().messages.pop().unwrap();
    assert_eq!(msg.delivered_count, 1);
    assert!(!msg.delivered);
    let stale = v[0].group_next(&gid, v[4].identity.pin()).unwrap().unwrap();
    v[0].group_delete_sent(&gid).unwrap();
    for b in 1..4 {
        route(&mut v, 0, b, &gid);
    }
    assert_eq!(progress(&v[0], &gid)["confirmed"], 3);
    assert_eq!(progress(&v[0], &gid)["total"], 4);
    assert_eq!(v[0].group_chat(&gid).unwrap().deletion, Some(false));
    assert_eq!(v[4].group_chat(&gid).unwrap().messages.len(), 2);
    let pin = v[0].identity.pin();
    let request = v[0].group_next(&gid, v[4].identity.pin()).unwrap().unwrap();
    let lost = v[4].apply_group(pin, request).unwrap().unwrap();
    assert_eq!(progress(&v[0], &gid)["confirmed"], 3);
    let pin = v[0].identity.pin();
    v[4].apply_group(pin, stale).unwrap();
    assert_eq!(v[4].group_chat(&gid).unwrap().messages.len(), 1);
    let recipient = v[4].identity.pin();
    v[0].apply_group(recipient, lost).unwrap();
    assert_eq!(v[0].group_chat(&gid).unwrap().deletion, Some(true));
    for member in &v {
        let chat = member.group_chat(&gid).unwrap();
        assert_eq!(chat.messages.len(), 1);
        assert_eq!(chat.messages[0].text, "preserve author one");
    }
    v[0].group_send(&gid, "after deletion").unwrap();
    sync(&mut v, &gid);
    for member in &v {
        assert_eq!(member.group_chat(&gid).unwrap().messages.len(), 2);
    }
}
#[test]
fn persisted_pending_deletion_retries_and_old_ack_cannot_complete_new_operation() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 3);
    let gid = create(&mut v);
    import(&mut v, &gid);
    sync(&mut v, &gid);
    v[0].group_send(&gid, "erase").unwrap();
    sync(&mut v, &gid);
    v[0].group_delete_sent(&gid).unwrap();
    let ack = route(&mut v, 0, 1, &gid).unwrap();
    v[0].group_delete_sent(&gid).unwrap();
    let pin = v[1].identity.pin();
    v[0].apply_group(pin, ack).unwrap();
    assert_eq!(progress(&v[0], &gid)["confirmed"], 0);
    let own = v[0].identity.pin();
    drop(v);
    let mut v: Vec<_> = (0..3)
        .map(|i| Vault::open(&root.path().join(i.to_string()), &[i as u8 + 1; 32], false).unwrap())
        .collect();
    assert_eq!(v[0].identity.pin(), own);
    assert_eq!(progress(&v[0], &gid)["confirmed"], 0);
    sync(&mut v, &gid);
    assert_eq!(progress(&v[0], &gid)["confirmed"], 2);
    for member in &v {
        assert!(member.group_chat(&gid).unwrap().messages.is_empty());
    }
}
#[test]
fn tls_author_context_membership_and_receipts_cannot_be_substituted() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 4);
    let gid = create(&mut v[..3]);
    import(&mut v[..3], &gid);
    sync(&mut v[..3], &gid);
    v[0].group_send(&gid, "source").unwrap();
    let p = v[0].group_next(&gid, v[1].identity.pin()).unwrap().unwrap();
    let mut forged = p.clone();
    forged.author = v[2].identity.pin();
    let pin = v[0].identity.pin();
    assert!(v[1].apply_group(pin, forged).is_err());
    let ack = v[1].apply_group(pin, p.clone()).unwrap().unwrap();
    let stranger = v[3].identity.pin();
    let mut wrong = p.clone();
    wrong.author = stranger;
    assert!(v[1].apply_group(stranger, wrong).is_err());
    let wrongpin = v[2].identity.pin();
    assert!(v[0].apply_group(wrongpin, ack.clone()).is_err());
    let pin = v[1].identity.pin();
    v[0].apply_group(pin, ack.clone()).unwrap();
    v[0].apply_group(pin, ack).unwrap();
    assert_eq!(
        v[0].group_chat(&gid).unwrap().messages[0].delivered_count,
        1
    );
    let mut wrong = p;
    wrong.version = 1;
    let pin = v[0].identity.pin();
    assert!(v[1].apply_from(pin, wrong).is_err());
}
#[test]
fn invites_are_immutable_idempotent_and_do_not_authorize_direct_conversations() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 4);
    let gid = create(&mut v[..3]);
    let code = v[0].group_invitation(&gid).unwrap().code().unwrap();
    assert!(v[3].preview_group(&code).is_err());
    assert!(v[3].import_group(&code).is_err());
    import(&mut v[..3], &gid);
    sync(&mut v[..3], &gid);
    v[1].group_send(&gid, "persist").unwrap();
    let before = serde_json::to_value(v[1].snapshot(None).unwrap()).unwrap();
    assert_eq!(v[1].import_group(&code).unwrap(), gid);
    assert_eq!(
        serde_json::to_value(v[1].snapshot(None).unwrap()).unwrap(),
        before
    );
    assert_eq!(v[1].direct_peers().unwrap().len(), 0);
    assert_eq!(v[1].peers().unwrap().len(), 2);
    let bc = hex::encode(psychotalk_core::vault::conversation_id(
        v[1].identity.pin(),
        v[2].identity.pin(),
    ));
    assert!(v[1].send_to(&bc, "no DM").is_err());
    let mut inv = Invitation::parse(&code).unwrap();
    inv.roster.name = "Altered".into();
    assert!(Invitation::parse(&inv.code().unwrap()).is_err());
    let target = v[2].identity.pin();
    v[1].group_rename_member(&gid, target, "@local").unwrap();
    assert_eq!(v[2].group_chat(&gid).unwrap().tagname, "Equipe fixa");
    let peer = crate::code(&v[2]);
    let direct = v[1].pair(&peer).unwrap();
    assert_eq!(direct, bc);
    v[1].send_to(&direct, "explicit DM").unwrap();
    assert_eq!(v[1].group_chat(&gid).unwrap().messages.len(), 1);
    let cid = v[0].direct_peers().unwrap()[0].id.clone();
    assert!(v[0].create_group("duplicate", &[cid.clone(), cid]).is_err());
}
#[test]
fn unknown_group_request_waits_without_acceptance_or_mutating_existing_dm() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 3);
    let gid = create(&mut v);
    let pin = v[0].identity.pin();
    let request = v[0].group_next(&gid, v[1].identity.pin()).unwrap().unwrap();
    let wait = v[1].apply_group(pin, request).unwrap().unwrap();
    assert!(matches!(
        wait.content,
        Content::Group {
            event: GroupEvent::Wait
        }
    ));
    assert!(!v[1].is_group(&gid).unwrap());
    let peer = code(&v[0]);
    let cid = v[1].pair(&peer).unwrap();
    v[1].send_to(&cid, "unblocked DM").unwrap();
    let p = v[1].next_for(&cid).unwrap();
    let pin = v[1].identity.pin();
    assert!(v[0].apply_from(pin, p).unwrap().is_some());
    assert_eq!(v[0].chat(&cid).unwrap().messages[0].text, "unblocked DM");
}
#[test]
fn migration_from_v2_preserves_cipher_identity_history_and_pending_deletion() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 3);
    let peer = code(&v[1]);
    let ab = v[0].pair(&peer).unwrap();
    v[0].send_to(&ab, "before").unwrap();
    v[0].delete_sent(&ab).unwrap();
    v[0].send_to(&ab, "after").unwrap();
    let pin = v[0].identity.pin();
    drop(v);
    let db = Connection::open(root.path().join("0")).unwrap();
    db.execute_batch(&format!("PRAGMA key=\"x'{}'\";DROP TABLE group_delete_acks;DROP TABLE group_deletions;DROP TABLE group_deliveries;DROP TABLE group_messages;DROP TABLE group_counters;DROP TABLE group_members;DROP TABLE groups;PRAGMA user_version=2;",hex::encode([1;32]))).unwrap();
    drop(db);
    let v = Vault::open(&root.path().join("0"), &[1; 32], false).unwrap();
    assert_eq!(v.identity.pin(), pin);
    assert_eq!(v.chat(&ab).unwrap().messages[0].text, "after");
    assert_eq!(v.chat(&ab).unwrap().deletion, Some(false));
    assert!(matches!(
        v.next_for(&ab).unwrap().content,
        Content::Delete { limit: 1, .. }
    ));
}

#[test]
fn maximum_utf8_group_frame_retransmission_conflicts_and_invalid_rosters() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 6);
    let cids: Vec<_> = (1..6)
        .map(|i| {
            let peer = code(&v[i]);
            v[0].pair(&peer).unwrap()
        })
        .collect();
    assert!(v[0].create_group("too small", &cids[..1]).is_err());
    assert!(v[0].create_group("too large", &cids).is_err());
    let gid = v[0].create_group("UTF-8", &cids[..2]).unwrap();
    import(&mut v[..3], &gid);
    sync(&mut v[..3], &gid);
    assert!(v[0].group_send(&gid, "").is_err());
    assert!(v[0].group_send(&gid, &"😀".repeat(1025)).is_err());
    let text = "😀".repeat(1024);
    v[0].group_send(&gid, &text).unwrap();
    let pin = v[0].identity.pin();
    let p = v[0].group_next(&gid, v[1].identity.pin()).unwrap().unwrap();
    let ack = v[1].apply_group(pin, p.clone()).unwrap().unwrap();
    let frame = serde_json::to_vec(&vec![ack, p.clone()]).unwrap();
    assert!(frame.len() <= psychotalk_core::transport::MAX_FRAME);
    v[1].apply_group(pin, p.clone()).unwrap();
    assert_eq!(v[1].group_chat(&gid).unwrap().messages[0].text, text);
    let mut conflicting = p.clone();
    if let Content::Group {
        event: GroupEvent::Text { ref mut id, .. },
    } = conflicting.content
    {
        *id = "e".repeat(64);
    }
    assert!(v[1].apply_group(pin, conflicting).is_err());
    assert_eq!(v[1].group_chat(&gid).unwrap().messages.len(), 1);
    let inv = v[0].group_invitation(&gid).unwrap();
    let mut invalid = inv.clone();
    invalid.roster.members.swap(0, 1);
    assert!(Invitation::parse(&invalid.code().unwrap()).is_err());
    assert!(Invitation::parse(&format!("{}x", inv.code().unwrap())).is_err());
    let valid = inv.code().unwrap();
    let mut outsider = Vault::open(&root.path().join("outsider"), &[8; 32], true).unwrap();
    assert!(outsider.import_group(&valid).is_err());
}

#[test]
fn separate_groups_with_same_roster_keep_histories_receipts_and_deletions_isolated() {
    let root = TempDir::new().unwrap();
    let mut v = nodes(&root, 3);
    let one = create(&mut v);
    import(&mut v, &one);
    sync(&mut v, &one);
    let cids: Vec<_> = v[0]
        .direct_peers()
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    let two = v[0].create_group("Second group", &cids).unwrap();
    assert_ne!(one, two);
    import(&mut v, &two);
    sync(&mut v, &two);
    for gid in [&one, &two] {
        v[0].group_send(gid, "own stream").unwrap();
        v[1].group_send(gid, "other author").unwrap();
        sync(&mut v, gid);
    }
    v[0].group_delete_sent(&one).unwrap();
    sync(&mut v, &one);
    for member in &v {
        assert_eq!(member.group_chat(&one).unwrap().messages.len(), 1);
        assert_eq!(member.group_chat(&two).unwrap().messages.len(), 2);
        assert_eq!(member.group_chat(&two).unwrap().deletion, None);
    }
    assert_eq!(v[0].group_chat(&one).unwrap().deletion, Some(true));
}
