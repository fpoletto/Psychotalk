//! Password envelopes protected by a separate OS credential, never a file fallback.
use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use anyhow::{Result, ensure};
use psychotalk_core::vault::{Vault, derive};
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
const OWNER: &[u8] = b"Psychotalk desktop vault v1\n";
const LENGTH: usize = 109;
pub trait SecretStore: Send + Sync {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>>;
    fn write(&self, key: &[u8]) -> Result<()>;
    fn remove(&self) -> Result<()>;
}
pub struct OsStore {
    entry: keyring::Entry,
}
impl OsStore {
    pub fn new(root: &Path) -> Result<Self> {
        let canonical = fs::canonicalize(root)?;
        let user = hex::encode(Sha256::digest(canonical.as_os_str().as_encoded_bytes()));
        Ok(Self {
            entry: keyring::Entry::new("app.psychotalk.desktop.v1", &user)?,
        })
    }
}
impl SecretStore for OsStore {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>> {
        match self.entry.get_secret() {
            Ok(key) => Ok(Some(Zeroizing::new(key))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => anyhow::bail!(
                "Não foi possível acessar o Keychain/Secret Service. Desbloqueie o chaveiro do sistema e tente novamente."
            ),
        }
    }
    fn write(&self, key: &[u8]) -> Result<()> {
        self.entry.set_secret(key).map_err(|_|anyhow::anyhow!("Não foi possível gravar no Keychain/Secret Service. Nenhuma chave será salva em arquivo."))
    }
    fn remove(&self) -> Result<()> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(_) => anyhow::bail!(
                "Não foi possível invalidar a chave no chaveiro; limpeza permanece pendente."
            ),
        };
        ensure!(self.read()?.is_none(), "key removal not confirmed");
        Ok(())
    }
}
pub fn private_dir(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "unsafe storage directory"
        );
    }
    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub struct Envelopes<S: SecretStore> {
    pub root: PathBuf,
    pub store: S,
}
pub enum Unlock {
    Normal(Zeroizing<Vec<u8>>),
    Cleanup,
    Invalid,
}
impl<S: SecretStore> Envelopes<S> {
    pub fn new(base: &Path, store: S) -> Result<Self> {
        private_dir(base)?;
        let root = base.join("vault");
        private_dir(&root)?;
        let owner = root.join(".owner");
        if !owner.exists() {
            ensure!(
                fs::read_dir(&root)?.next().is_none(),
                "unrecognized vault directory; no files were removed"
            );
            let mut f = OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&owner)?;
            f.write_all(OWNER)?;
            f.sync_all()?;
            File::open(&root)?.sync_all()?;
        }
        ensure!(fs::read(owner)? == OWNER, "unrecognized vault directory");
        Ok(Self { root, store })
    }
    fn atomic(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let temp = self.root.join(format!(".{name}.next"));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(temp, self.root.join(name))?;
        self.sync()
    }
    fn sync(&self) -> Result<()> {
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }
    pub fn configured(&self) -> bool {
        self.root.join("open.env").is_file()
            && self.root.join("clean.env").is_file()
            && self.root.join("vault.db").is_file()
    }
    pub fn recover(&self) -> Result<()> {
        if self.root.join("wipe.intent").exists() || self.root.join("setup.intent").exists() {
            self.wipe()?;
        }
        ensure!(
            self.configured()
                || fs::read_dir(&self.root)?.all(|e| e.is_ok_and(|e| e.file_name() == ".owner")),
            "Armazenamento incompleto. A rede permanece bloqueada."
        );
        Ok(())
    }
    pub fn setup(&self, normal: &[u8], cleanup: &[u8]) -> Result<()> {
        self.recover()?;
        ensure!(
            !self.configured()
                && !normal.is_empty()
                && !cleanup.is_empty()
                && normal.len() <= 1024
                && cleanup.len() <= 1024
                && normal != cleanup,
            "Senhas devem ser distintas, não vazias e ter até 1024 bytes."
        );
        ensure!(
            self.store.read()?.is_none(),
            "Há uma chave órfã no chaveiro; não será substituída automaticamente."
        );
        self.atomic("setup.intent", &[1])?;
        let mut key = Zeroizing::new(vec![0; 32]);
        let mut dek = Zeroizing::new(vec![0; 32]);
        let mut token = Zeroizing::new(vec![0; 32]);
        OsRng.fill_bytes(&mut key);
        OsRng.fill_bytes(&mut dek);
        OsRng.fill_bytes(&mut token);
        self.store.write(&key)?;
        ensure!(
            self.store.read()?.as_deref().map(|k| k.as_slice()) == Some(key.as_slice()),
            "keychain verification failed"
        );
        self.seal("open.env", normal, 1, &dek, &key)?;
        self.seal("clean.env", cleanup, 2, &token, &key)?;
        drop(Vault::open(&self.root.join("vault.db"), &dek, true)?);
        fs::set_permissions(
            self.root.join("vault.db"),
            fs::Permissions::from_mode(0o600),
        )?;
        fs::remove_file(self.root.join("setup.intent"))?;
        self.sync()
    }
    fn seal(&self, name: &str, password: &[u8], kind: u8, data: &[u8], key: &[u8]) -> Result<()> {
        let mut salt = [0; 16];
        OsRng.fill_bytes(&mut salt);
        let mut header = b"PTD1".to_vec();
        header.extend(salt);
        let derived = derive(password, &salt)?;
        let mut payload = Zeroizing::new(vec![kind]);
        payload.extend_from_slice(data);
        let inner = Zeroizing::new(encrypt(&derived[..], &header, &payload)?);
        let outer = encrypt(key, &header, &inner)?;
        header.extend(outer);
        self.atomic(name, &header)
    }
    fn unseal(
        &self,
        name: &str,
        password: &[u8],
        kind: u8,
        key: Option<&[u8]>,
    ) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let bytes =
            Zeroizing::new(fs::read(self.root.join(name)).unwrap_or_else(|_| vec![0; LENGTH]));
        let valid = bytes.len() == LENGTH && bytes.starts_with(b"PTD1");
        let salt = if valid { &bytes[4..20] } else { &[0u8; 16] };
        let derived = derive(password, salt)?;
        if !valid {
            return Ok(None);
        }
        let Some(key) = key else { return Ok(None) };
        let Some(outer) = decrypt(key, &bytes[..20], &bytes[20..]) else {
            return Ok(None);
        };
        let Some(payload) = decrypt(&derived[..], &bytes[..20], &outer) else {
            return Ok(None);
        };
        if payload.len() != 33 || payload[0] != kind {
            return Ok(None);
        }
        Ok(Some(Zeroizing::new(payload[1..].to_vec())))
    }
    pub fn unlock(&self, password: &[u8]) -> Result<Unlock> {
        ensure!(password.len() <= 1024, "password too long");
        let key = self.store.read()?;
        let key = key.as_ref().map(|k| k.as_slice());
        let normal = self.unseal("open.env", password, 1, key)?;
        let cleanup = self.unseal("clean.env", password, 2, key)?;
        Ok(if cleanup.is_some() {
            Unlock::Cleanup
        } else if let Some(key) = normal {
            Unlock::Normal(key)
        } else {
            Unlock::Invalid
        })
    }
    /// Caller must revoke/close all sessions before invoking durable cleanup.
    pub fn wipe(&self) -> Result<()> {
        ensure!(
            fs::read(self.root.join(".owner"))? == OWNER,
            "unrecognized vault directory"
        );
        if !self.root.join("wipe.intent").exists() {
            self.atomic("wipe.intent", &[1])?;
        }
        self.store.remove()?;
        for item in fs::read_dir(&self.root)? {
            let item = item?;
            if item.file_name() == ".owner" || item.file_name() == "wipe.intent" {
                continue;
            }
            if item.file_type()?.is_dir() {
                fs::remove_dir_all(item.path())?;
            } else {
                fs::remove_file(item.path())?;
            }
        }
        self.sync()?;
        fs::remove_file(self.root.join("wipe.intent"))?;
        self.sync()
    }
}
fn encrypt(key: &[u8], aad: &[u8], message: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow::anyhow!("invalid key"))?;
    let mut nonce = [0; 12];
    OsRng.fill_bytes(&mut nonce);
    let mut out = nonce.to_vec();
    out.extend(
        cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: message, aad })
            .map_err(|_| anyhow::anyhow!("envelope encryption failed"))?,
    );
    Ok(out)
}
fn decrypt(key: &[u8], aad: &[u8], bytes: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if bytes.len() < 28 {
        return None;
    }
    Aes256Gcm::new_from_slice(key)
        .ok()?
        .decrypt(
            Nonce::from_slice(&bytes[..12]),
            Payload {
                msg: &bytes[12..],
                aad,
            },
        )
        .ok()
        .map(Zeroizing::new)
}
