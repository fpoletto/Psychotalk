//! Opt-in: uses a disposable real OS keyring item, never a file or memory substitute.
use psychotalk_desktop::envelopes::{Envelopes, OsStore, SecretStore, Unlock};
#[test]
fn real_system_keyring_roundtrip_reopen_and_cleanup() {
    if std::env::var("PSYCHOTALK_TEST_KEYRING").as_deref() != Ok("1") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let e = Envelopes::new(dir.path(), OsStore::new(dir.path()).unwrap()).unwrap();
    e.setup(b"os normal", b"os cleanup").unwrap();
    let Unlock::Normal(key) = e.unlock(b"os normal").unwrap() else {
        panic!()
    };
    drop(key);
    let reopened = Envelopes::new(dir.path(), OsStore::new(dir.path()).unwrap()).unwrap();
    assert!(matches!(
        reopened.unlock(b"os normal").unwrap(),
        Unlock::Normal(_)
    ));
    reopened.wipe().unwrap();
    assert!(reopened.store.read().unwrap().is_none());
}
