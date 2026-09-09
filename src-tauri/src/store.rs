use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::crypto::aead::VaultKey;
use crate::error::{Result, WalletError};

/// File header. The version byte lets a later build migrate an old vault
/// instead of guessing at its layout.
const MAGIC: &[u8; 4] = b"SFV\x01";

/// One named bucket. Balances cross the IPC bridge as decimal strings in the
/// asset's smallest unit, because a JS number cannot hold a satoshi count or a
/// Monero atomic-unit amount without silently rounding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub id: String,
    pub name: String,
    pub asset: String,
    pub balance_minor: String,
    pub address_count: u32,
}

/// Everything held encrypted at rest. SMTP credentials and settings land here
/// too once those features exist; nothing about this wallet is stored in the
/// clear.
// No Debug derive: this holds the mnemonic, and a stray dbg! or a panic
// message would put it somewhere it can be read.
#[derive(Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct VaultPayload {
    pub mnemonic: String,
    #[zeroize(skip)]
    pub buckets: Vec<Bucket>,
}

pub fn exists(path: &Path) -> bool {
    path.is_file()
}

pub fn write(path: &Path, key: &VaultKey, payload: &VaultPayload) -> Result<()> {
    let json = Zeroizing::new(
        serde_json::to_vec(payload).map_err(|e| WalletError::Storage(e.to_string()))?,
    );
    let sealed = key.seal(&json)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| WalletError::Storage(e.to_string()))?;
    }

    let mut blob = Vec::with_capacity(MAGIC.len() + sealed.len());
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&sealed);

    // Write beside the target and rename, so a crash mid-write leaves the
    // previous vault intact rather than a truncated one.
    let temp = path.with_extension("vault.tmp");
    fs::write(&temp, &blob).map_err(|e| WalletError::Storage(e.to_string()))?;
    fs::rename(&temp, path).map_err(|e| WalletError::Storage(e.to_string()))?;
    Ok(())
}

pub fn read(path: &Path, key: &VaultKey) -> Result<VaultPayload> {
    let blob = fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => WalletError::NoVault,
        _ => WalletError::Storage(e.to_string()),
    })?;

    if blob.len() <= MAGIC.len() || &blob[..MAGIC.len()] != MAGIC {
        return Err(WalletError::Decrypt);
    }

    let plaintext = key.open(&blob[MAGIC.len()..])?;
    serde_json::from_slice(&plaintext).map_err(|_| WalletError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("sf-test-{}", std::process::id()));
        let path = dir.join("wallet.vault");
        let key = VaultKey::random();

        let payload = VaultPayload {
            mnemonic: "phrase goes here".into(),
            buckets: vec![Bucket {
                id: "b1".into(),
                name: "Rent".into(),
                asset: "BTC".into(),
                balance_minor: "125000".into(),
                address_count: 2,
            }],
        };

        write(&path, &key, &payload).unwrap();
        let back = read(&path, &key).unwrap();
        assert_eq!(back.mnemonic, "phrase goes here");
        assert_eq!(back.buckets.len(), 1);
        assert_eq!(back.buckets[0].balance_minor, "125000");

        // A different key must not open it.
        assert!(read(&path, &VaultKey::random()).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_a_foreign_file() {
        let dir = std::env::temp_dir().join(format!("sf-magic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wallet.vault");
        fs::write(&path, b"not a vault at all").unwrap();
        assert!(read(&path, &VaultKey::random()).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
