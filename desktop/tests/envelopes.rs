use anyhow::Result;
use psychotalk_desktop::envelopes::{Envelopes, SecretStore, Unlock};
use std::{fs, sync::Mutex};
use zeroize::Zeroizing;
#[derive(Default)]
struct Memory(Mutex<Option<Vec<u8>>>);
impl SecretStore for Memory {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>> {
        Ok(self.0.lock().unwrap().clone().map(Zeroizing::new))
    }
    fn write(&self, k: &[u8]) -> Result<()> {
        *self.0.lock().unwrap() = Some(k.to_vec());
        Ok(())
    }
    fn remove(&self) -> Result<()> {
        self.0.lock().unwrap().take();
        Ok(())
    }
}
#[test]
fn real_sqlcipher_two_envelopes_wrong_password_and_keychain_dependency() {
    let t = tempfile::tempdir().unwrap();
    let e = Envelopes::new(t.path(), Memory::default()).unwrap();
    e.setup(b"normal", b"cleanup").unwrap();
    assert_eq!(fs::read(e.root.join("open.env")).unwrap().len(), 109);
    assert!(matches!(e.unlock(b"wrong").unwrap(), Unlock::Invalid));
    let Unlock::Normal(key) = e.unlock(b"normal").unwrap() else {
        panic!()
    };
    let v = psychotalk_core::vault::Vault::open(&e.root.join("vault.db"), &key, false).unwrap();
    let pin = v.identity.pin();
    drop(v);
    let bytes = fs::read(e.root.join("vault.db")).unwrap();
    assert_ne!(&bytes[..16], b"SQLite format 3\0");
    let Unlock::Normal(key) = e.unlock(b"normal").unwrap() else {
        panic!()
    };
    assert_eq!(
        psychotalk_core::vault::Vault::open(&e.root.join("vault.db"), &key, false)
            .unwrap()
            .identity
            .pin(),
        pin
    );
    e.store.remove().unwrap();
    assert!(matches!(e.unlock(b"normal").unwrap(), Unlock::Invalid));
}
#[test]
fn cleanup_without_database_and_interrupted_setup_recovery() {
    let t = tempfile::tempdir().unwrap();
    let e = Envelopes::new(t.path(), Memory::default()).unwrap();
    e.setup(b"normal", b"cleanup").unwrap();
    fs::remove_file(e.root.join("vault.db")).unwrap();
    assert!(matches!(e.unlock(b"cleanup").unwrap(), Unlock::Cleanup));
    e.wipe().unwrap();
    assert!(!e.configured());
    assert!(e.store.read().unwrap().is_none());
    e.setup(b"normal2", b"cleanup2").unwrap();
    fs::write(e.root.join("setup.intent"), [1]).unwrap();
    e.recover().unwrap();
    assert!(!e.configured());
    assert!(e.root.join(".owner").is_file());
}
#[test]
fn unrecognized_directory_and_symlink_cannot_be_wiped() {
    let t = tempfile::tempdir().unwrap();
    let vault = t.path().join("vault");
    fs::create_dir(&vault).unwrap();
    fs::write(vault.join("precious"), b"keep").unwrap();
    assert!(Envelopes::new(t.path(), Memory::default()).is_err());
    assert!(vault.join("precious").exists());
    let other = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(&vault, other.path().join("vault")).unwrap();
    assert!(Envelopes::new(other.path(), Memory::default()).is_err());
}
