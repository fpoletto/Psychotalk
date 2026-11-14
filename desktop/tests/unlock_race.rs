use anyhow::Result;
use psychotalk_desktop::{
    engine::Engine,
    envelopes::{Envelopes, SecretStore},
};
use std::sync::{Arc, Condvar, Mutex};
use zeroize::Zeroizing;
#[derive(Default)]
struct State {
    key: Option<Vec<u8>>,
    block: bool,
    entered: bool,
    release: bool,
}
#[derive(Default)]
struct BlockingStore {
    state: Mutex<State>,
    cv: Condvar,
}
impl SecretStore for BlockingStore {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let mut s = self.state.lock().unwrap();
        if s.block && !s.release {
            s.entered = true;
            self.cv.notify_all();
            while !s.release {
                s = self.cv.wait(s).unwrap();
            }
        }
        Ok(s.key.clone().map(Zeroizing::new))
    }
    fn write(&self, k: &[u8]) -> Result<()> {
        self.state.lock().unwrap().key = Some(k.to_vec());
        Ok(())
    }
    fn remove(&self) -> Result<()> {
        self.state.lock().unwrap().key = None;
        Ok(())
    }
}
#[test]
fn hidden_window_during_unlock_cannot_start_a_session_after_revoke() {
    let dir = tempfile::tempdir().unwrap();
    let e = Envelopes::new(dir.path(), BlockingStore::default()).unwrap();
    e.setup(b"normal", b"cleanup").unwrap();
    e.store.state.lock().unwrap().block = true;
    let engine = Arc::new(Engine::new(e, dir.path().join("must-never-start-tor")).unwrap());
    let task = engine.clone();
    let thread = std::thread::spawn(move || task.unlock(b"normal", None));
    {
        let store = &engine.envelopes.store;
        let mut s = store.state.lock().unwrap();
        while !s.entered {
            s = store.cv.wait(s).unwrap();
        }
    }
    engine.revoke();
    {
        let store = &engine.envelopes.store;
        store.state.lock().unwrap().release = true;
        store.cv.notify_all();
    }
    assert!(
        thread
            .join()
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("ocultada")
    );
    assert!(engine.active().is_none());
    engine.envelopes.wipe().unwrap();
}
