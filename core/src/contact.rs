use anyhow::{Result, ensure};
use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Contact {
    pub onion: String,
    pub pin: [u8; 32],
}

impl Contact {
    pub fn code(&self) -> String {
        format!("psychotalk:1:{}:{}", self.onion, hex::encode(self.pin))
    }

    pub fn parse(code: &str) -> Result<Self> {
        ensure!(code.len() < 180, "invalid contact");
        let p: Vec<_> = code.trim().split(':').collect();
        ensure!(
            p.len() == 4 && p[0] == "psychotalk" && p[1] == "1",
            "invalid contact"
        );
        validate_onion(p[2])?;
        let pin: [u8; 32] = hex::decode(p[3])?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid pin"))?;
        Ok(Self {
            onion: p[2].to_owned(),
            pin,
        })
    }
}

pub fn validate_onion(host: &str) -> Result<()> {
    ensure!(
        host.len() == 62 && host.ends_with(".onion"),
        "onion v3 required"
    );
    let stem = &host[..56];
    ensure!(
        stem.bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)),
        "invalid onion"
    );
    let raw = BASE32_NOPAD.decode(stem.to_ascii_uppercase().as_bytes())?;
    ensure!(raw.len() == 35 && raw[34] == 3, "onion v3 required");
    let mut h = Sha3_256::new();
    h.update(b".onion checksum");
    h.update(&raw[..32]);
    h.update([3]);
    ensure!(raw[32..34] == h.finalize()[..2], "invalid onion checksum");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_direct_destinations() {
        for s in [
            "127.0.0.1",
            "localhost",
            "192.168.1.1",
            "example.com",
            "::1",
            &format!("{}.onion", "a".repeat(56)),
        ] {
            assert!(validate_onion(s).is_err());
        }
    }
}
