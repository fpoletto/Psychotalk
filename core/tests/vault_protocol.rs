use psychotalk_core::{
    contact::Contact,
    vault::{Content, Packet, Vault},
};
use sha3::{Digest, Sha3_256};
use tempfile::TempDir;
fn onion() -> String {
    let key = [7u8; 32];
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
fn pair() -> (TempDir, Vault, Vault) {
    let d = TempDir::new().unwrap();
    let mut a = Vault::open(&d.path().join("a"), &[1; 32], true).unwrap();
    let mut b = Vault::open(&d.path().join("b"), &[2; 32], true).unwrap();
    a.pair(
        &Contact {
            onion: onion(),
            pin: b.identity.pin(),
        }
        .code(),
    )
    .unwrap();
    b.pair(
        &Contact {
            onion: onion(),
            pin: a.identity.pin(),
        }
        .code(),
    )
    .unwrap();
    (d, a, b)
}
fn deliver(a: &mut Vault, b: &mut Vault) -> Packet {
    let p = a.next().unwrap();
    let ack = b.apply(p.clone()).unwrap().unwrap();
    a.apply(ack).unwrap();
    p
}
#[test]
fn actual_cipher_and_wrong_key() {
    let (d, mut a, _) = pair();
    a.send("sensitive test phrase 9381").unwrap();
    drop(a);
    let bytes = std::fs::read(d.path().join("a")).unwrap();
    assert!(!bytes.starts_with(b"SQLite format 3"));
    assert!(
        !bytes
            .windows(26)
            .any(|w| w == b"sensitive test phrase 9381")
    );
    assert!(Vault::open(&d.path().join("a"), &[3; 32], false).is_err());
    assert!(Vault::open(&d.path().join("a"), &[1; 32], false).is_ok());
}
#[test]
fn offline_delete_ack_loss_reopen_late_message_and_new_message() {
    let (d, mut a, mut b) = pair();
    a.send("old").unwrap();
    let old = deliver(&mut a, &mut b);
    b.send("keep peer text").unwrap();
    deliver(&mut b, &mut a);
    a.delete_all().unwrap();
    assert_eq!(a.view().unwrap().messages.len(), 1);
    assert_eq!(b.view().unwrap().messages.len(), 2);
    drop(a);
    let mut a = Vault::open(&d.path().join("a"), &[1; 32], false).unwrap();
    assert_eq!(a.view().unwrap().deletion, Some(false));
    let deletion = a.next().unwrap();
    let lost_ack = b.apply(deletion.clone()).unwrap().unwrap();
    drop(b);
    let mut b = Vault::open(&d.path().join("b"), &[2; 32], false).unwrap();
    b.apply(old).unwrap();
    assert_eq!(b.view().unwrap().messages.len(), 1);
    let retry = b.apply(deletion).unwrap().unwrap();
    a.apply(retry).unwrap();
    a.apply(lost_ack).unwrap();
    assert_eq!(a.view().unwrap().deletion, Some(true));
    a.send("new").unwrap();
    let p = deliver(&mut a, &mut b);
    assert!(matches!(p.content, Content::Message { seq: 2, .. }));
    assert_eq!(b.view().unwrap().messages.len(), 2);
}
#[test]
fn duplicate_conflict_and_authorship_tamper() {
    let (_directory, mut a, mut b) = pair();
    a.send("once").unwrap();
    let p = deliver(&mut a, &mut b);
    b.apply(p.clone()).unwrap();
    assert_eq!(b.view().unwrap().messages.len(), 1);
    let mut fake = p.clone();
    fake.author = b.identity.pin();
    assert!(b.apply(fake).is_err());
    let mut fake = p.clone();
    fake.conversation[0] ^= 1;
    assert!(b.apply(fake).is_err());
    let mut fake = p;
    if let Content::Message { ref mut id, .. } = fake.content {
        *id = "a".repeat(32);
    }
    assert!(b.apply(fake).is_err());
    assert!(
        b.pair(
            &Contact {
                onion: onion(),
                pin: a.identity.pin()
            }
            .code()
        )
        .is_err()
    );
}
#[test]
fn successive_deletes_never_regress_and_old_ack_does_not_confirm_new() {
    let (_directory, mut a, mut b) = pair();
    a.send("one").unwrap();
    let first = deliver(&mut a, &mut b);
    a.delete_all().unwrap();
    let d1 = a.next().unwrap();
    let ack1 = b.apply(d1.clone()).unwrap().unwrap();
    a.send("two").unwrap();
    a.delete_all().unwrap();
    let d2 = a.next().unwrap();
    a.apply(ack1).unwrap();
    assert_eq!(a.view().unwrap().deletion, Some(false));
    let ack2 = b.apply(d2).unwrap().unwrap();
    a.apply(ack2).unwrap();
    b.apply(d1).unwrap();
    b.apply(first).unwrap();
    assert!(b.view().unwrap().messages.is_empty());
    a.send("three").unwrap();
    assert!(matches!(
        a.next().unwrap().content,
        Content::Message { seq: 3, .. }
    ));
}
#[test]
fn false_ack_demonstrates_trust_limit() {
    let (_directory, mut a, mut b) = pair();
    a.send("malicious client keeps this").unwrap();
    deliver(&mut a, &mut b);
    a.delete_all().unwrap();
    if let Content::Delete { id, limit } = &a.next().unwrap().content {
        a.apply(
            b.packet(Content::Deleted {
                id: id.clone(),
                limit: *limit,
            })
            .unwrap(),
        )
        .unwrap();
    } else {
        panic!();
    }
    assert_eq!(a.view().unwrap().deletion, Some(true));
    assert_eq!(b.view().unwrap().messages.len(), 1);
}
#[test]
fn old_ack_does_not_confirm_new_operation_at_same_limit() {
    let (_directory, mut a, mut b) = pair();
    a.send("one").unwrap();
    deliver(&mut a, &mut b);
    a.delete_all().unwrap();
    let old = b.apply(a.next().unwrap()).unwrap().unwrap();
    a.delete_all().unwrap();
    a.apply(old).unwrap();
    assert_eq!(a.view().unwrap().deletion, Some(false));
    let ack = b.apply(a.next().unwrap()).unwrap().unwrap();
    a.apply(ack).unwrap();
    assert_eq!(a.view().unwrap().deletion, Some(true));
}
#[test]
fn utf8_limit_and_real_wire_encoding_roundtrip() {
    let (_directory, mut a, mut b) = pair();
    let text = "😀".repeat(1024);
    a.send(&text).unwrap();
    assert!(a.send(&(text + "x")).is_err());
    let bytes = serde_json::to_vec(&vec![a.next().unwrap()]).unwrap();
    assert!(bytes.len() <= psychotalk_core::transport::MAX_FRAME);
    let mut decoded: Vec<Packet> = serde_json::from_slice(&bytes).unwrap();
    let ack = b.apply(decoded.pop().unwrap()).unwrap().unwrap();
    a.apply(ack).unwrap();
    assert_eq!(b.view().unwrap().messages[0].text.len(), 4096);
}
