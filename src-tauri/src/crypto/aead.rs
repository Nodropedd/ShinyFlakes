use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use rand::RngCore;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::{Result, WalletError};

pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// AES-256-GCM key for everything stored locally: the seed, SMTP credentials,
/// bucket metadata, settings. Held only for the duration of a single vault
/// read or write, then dropped and zeroed.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct VaultKey([u8; KEY_LEN]);

impl VaultKey {
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    pub fn random() -> Self {
        let mut bytes = [0u8; KEY_LEN];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Only the keychain module calls this, to hand the key to the OS store.
    pub fn expose(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.0))
    }

    /// Returns nonce || ciphertext || tag. A fresh random nonce per call, so
    /// rewriting the vault never reuses one under the same key.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self
            .cipher()
            .encrypt(nonce, plaintext)
            .map_err(|_| WalletError::Decrypt)?;

        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub fn open(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        if blob.len() <= NONCE_LEN {
            return Err(WalletError::Decrypt);
        }
        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let plaintext = self
            .cipher()
            .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
            .map_err(|_| WalletError::Decrypt)?;
        Ok(Zeroizing::new(plaintext))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let key = VaultKey::random();
        let sealed = key.seal(b"bucket metadata").unwrap();
        assert_eq!(&sealed[NONCE_LEN..].len(), &(b"bucket metadata".len() + 16));
        assert_eq!(&key.open(&sealed).unwrap()[..], b"bucket metadata");
    }

    #[test]
    fn rejects_tampering() {
        let key = VaultKey::random();
        let mut sealed = key.seal(b"bucket metadata").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(key.open(&sealed).is_err());
    }

    #[test]
    fn rejects_a_different_key() {
        let sealed = VaultKey::random().seal(b"bucket metadata").unwrap();
        assert!(VaultKey::random().open(&sealed).is_err());
    }
}
