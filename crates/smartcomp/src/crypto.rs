//! AES-256-GCM in 1 MiB chunks, key from Argon2id. Nonce = (blob id, chunk index),
//! so every chunk of every blob is encrypted under a unique nonce.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Result};
use argon2::{Algorithm, Argon2, Params, Version};

pub const CHUNK: usize = 1 << 20;
pub const TAG: usize = 16;

pub struct Cipher(Aes256Gcm);

impl Cipher {
    pub fn derive(password: &str, salt: &[u8; 32]) -> Result<Self> {
        let params = Params::new(64 * 1024, 3, 1, Some(32)).map_err(|e| anyhow!("argon2 params: {e}"))?;
        let mut key = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| anyhow!("key derivation failed: {e}"))?;
        Ok(Cipher(Aes256Gcm::new((&key).into())))
    }

    fn nonce(id: u32, idx: u64) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[..4].copy_from_slice(&id.to_le_bytes());
        n[4..].copy_from_slice(&idx.to_le_bytes());
        n
    }

    pub fn encrypt(&self, id: u32, plain: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(stored_len(plain.len(), true));
        // An empty blob still gets one (empty) chunk so it is authenticated.
        let chunks: Vec<&[u8]> = if plain.is_empty() { vec![plain] } else { plain.chunks(CHUNK).collect() };
        for (i, c) in chunks.into_iter().enumerate() {
            let n = Self::nonce(id, i as u64);
            let ct = self.0.encrypt(Nonce::from_slice(&n), c).map_err(|_| anyhow!("encryption failed"))?;
            out.extend_from_slice(&ct);
        }
        Ok(out)
    }

    pub fn decrypt(&self, id: u32, stored: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(stored.len());
        for (i, c) in stored.chunks(CHUNK + TAG).enumerate() {
            let n = Self::nonce(id, i as u64);
            let pt = self
                .0
                .decrypt(Nonce::from_slice(&n), c)
                .map_err(|_| anyhow!("authentication failed: wrong password or corrupted archive"))?;
            out.extend_from_slice(&pt);
        }
        Ok(out)
    }
}

pub fn stored_len(plain: usize, encrypted: bool) -> usize {
    if !encrypted {
        return plain;
    }
    plain + plain.div_ceil(CHUNK).max(1) * TAG
}
