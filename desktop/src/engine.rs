use crate::envelopes::{Envelopes, OsStore, SecretStore, Unlock};
use anyhow::{Result, ensure};
use psychotalk_core::session::{Session, Shared};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
pub struct Engine<S: SecretStore = OsStore> {
    pub envelopes: Envelopes<S>,
    tor: PathBuf,
    lifecycle: Mutex<()>,
    session: Mutex<Option<Session>>,
    attempts: Mutex<(u32, Instant)>,
    generation: AtomicU64,
}
impl<S: SecretStore> Engine<S> {
    pub fn new(envelopes: Envelopes<S>, tor: PathBuf) -> Result<Self> {
        envelopes.recover()?;
        Ok(Self {
            envelopes,
            tor,
            lifecycle: Mutex::new(()),
            session: Mutex::new(None),
            attempts: Mutex::new((0, Instant::now())),
            generation: AtomicU64::new(0),
        })
    }
    pub fn active(&self) -> Option<Arc<Shared>> {
        self.session
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.shared.clone())
    }
    pub fn status(&self) -> Value {
        json!({"configured":self.envelopes.configured(),"unlocked":self.active().is_some(),"version":env!("CARGO_PKG_VERSION"),"keyring":if cfg!(target_os="macos"){"Keychain do macOS"}else{"Secret Service do Linux"}})
    }
    fn close(&self) {
        if let Some(s) = self.session.lock().unwrap().take() {
            s.close();
        }
    }
    pub fn revoke(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(s) = self.active() {
            s.revoke();
        }
    }
    pub fn lock(&self) {
        self.revoke();
        let _guard = self.lifecycle.lock().unwrap();
        self.close();
    }
    pub fn unlock(&self, password: &[u8], cleanup: Option<&[u8]>) -> Result<Value> {
        let generation = self.generation.load(Ordering::SeqCst);
        let _guard = self.lifecycle.lock().unwrap();
        ensure!(self.active().is_none(), "Sessão já aberta.");
        {
            let a = self.attempts.lock().unwrap();
            ensure!(
                Instant::now() >= a.1,
                "Aguarde alguns segundos antes de tentar novamente."
            );
        }
        self.envelopes.recover()?;
        if let Some(other) = cleanup {
            self.envelopes.setup(password, other)?;
        }
        match self.envelopes.unlock(password)? {
            Unlock::Normal(key) => {
                let mut slot = self.session.lock().unwrap();
                ensure!(
                    self.generation.load(Ordering::SeqCst) == generation,
                    "A janela foi ocultada; desbloqueie novamente."
                );
                let session = Session::open(&self.envelopes.root, &self.tor, &key, false)?;
                *slot = Some(session);
                drop(slot);
                *self.attempts.lock().unwrap() = (0, Instant::now());
                Ok(self.status())
            }
            Unlock::Cleanup => {
                self.close();
                self.envelopes.wipe()?;
                Ok(json!({"cleaned":true,"configured":false,"unlocked":false}))
            }
            Unlock::Invalid => {
                let mut a = self.attempts.lock().unwrap();
                a.0 = a.0.saturating_add(1);
                a.1 = Instant::now() + Duration::from_secs(u64::from(a.0.min(10)));
                anyhow::bail!("Senha incorreta ou chave do sistema indisponível.")
            }
        }
    }
    pub fn heartbeat(&self) -> Result<()> {
        self.active()
            .ok_or_else(|| anyhow::anyhow!("locked"))?
            .heartbeat();
        Ok(())
    }
    pub fn command(&self, input: &str) -> Result<Value> {
        self.active()
            .ok_or_else(|| anyhow::anyhow!("locked"))?
            .command(input)
    }
    pub fn data_path(&self) -> &Path {
        &self.envelopes.root
    }
}
impl<S: SecretStore> Drop for Engine<S> {
    fn drop(&mut self) {
        if let Ok(slot) = self.session.get_mut()
            && let Some(s) = slot.take()
        {
            s.close();
        }
    }
}
