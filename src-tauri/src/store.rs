//! Encrypted vault file.

use std::fs;
use std::path::Path;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::crypto::aead::{VaultKey, KEY_LEN};
use crate::error::{Result, WalletError};

const MAGIC_V1: &[u8; 4] = b"SFV\x01";

const MAGIC_V2: &[u8; 4] = b"SFV\x02";
const SALT_LEN: usize = 16;
const HEADER_LEN: usize = 4 + 1 + SALT_LEN;

pub struct LockInfo {
    pub needs_passphrase: bool,
    salt: [u8; SALT_LEN],
}

fn derive(keychain: &VaultKey, passphrase: Option<&str>, salt: &[u8; SALT_LEN]) -> Result<VaultKey> {
    let Some(pass) = passphrase else {
        return Ok(VaultKey::from_bytes(*keychain.expose()));
    };

    let mut stretched = Zeroizing::new([0u8; KEY_LEN]);
    argon2::Argon2::default()
        .hash_password_into(pass.as_bytes(), salt, stretched.as_mut_slice())
        .map_err(|e| WalletError::Storage(format!("key derivation: {e}")))?;

    let mut mac = <Hmac<Sha256>>::new_from_slice(keychain.expose())
        .map_err(|e| WalletError::Storage(e.to_string()))?;
    mac.update(stretched.as_slice());

    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&mac.finalize().into_bytes());
    Ok(VaultKey::from_bytes(key))
}

#[derive(Default, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct VaultPayload {
    pub mnemonic: String,
}

pub fn exists(path: &Path) -> bool {
    path.is_file()
}

pub fn lock_info(path: &Path) -> Result<LockInfo> {
    let blob = fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => WalletError::NoVault,
        _ => WalletError::Storage(e.to_string()),
    })?;

    if blob.len() >= HEADER_LEN && &blob[..4] == MAGIC_V2 {
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&blob[5..5 + SALT_LEN]);
        return Ok(LockInfo {
            needs_passphrase: blob[4] == 1,
            salt,
        });
    }

    Ok(LockInfo {
        needs_passphrase: false,
        salt: [0u8; SALT_LEN],
    })
}

pub fn write(
    path: &Path,
    keychain: &VaultKey,
    passphrase: Option<&str>,
    payload: &VaultPayload,
) -> Result<()> {
    use rand::RngCore;
    let mut salt = [0u8; SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);

    let key = derive(keychain, passphrase, &salt)?;

    let json = Zeroizing::new(
        serde_json::to_vec(payload).map_err(|e| WalletError::Storage(e.to_string()))?,
    );
    let sealed = key.seal(&json)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| WalletError::Storage(e.to_string()))?;
    }

    let mut blob = Vec::with_capacity(HEADER_LEN + sealed.len());
    blob.extend_from_slice(MAGIC_V2);
    blob.push(if passphrase.is_some() { 1 } else { 0 });
    blob.extend_from_slice(&salt);
    blob.extend_from_slice(&sealed);

    let temp = path.with_extension("vault.tmp");
    fs::write(&temp, &blob).map_err(|e| WalletError::Storage(e.to_string()))?;
    fs::rename(&temp, path).map_err(|e| WalletError::Storage(e.to_string()))?;
    Ok(())
}

pub fn read(path: &Path, keychain: &VaultKey, passphrase: Option<&str>) -> Result<VaultPayload> {
    let blob = fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => WalletError::NoVault,
        _ => WalletError::Storage(e.to_string()),
    })?;

    let (key, body) = if blob.len() >= HEADER_LEN && &blob[..4] == MAGIC_V2 {
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&blob[5..5 + SALT_LEN]);

        let pass = if blob[4] == 1 { passphrase } else { None };
        (derive(keychain, pass, &salt)?, &blob[HEADER_LEN..])
    } else if blob.len() > MAGIC_V1.len() && &blob[..4] == MAGIC_V1 {

        (VaultKey::from_bytes(*keychain.expose()), &blob[MAGIC_V1.len()..])
    } else {
        return Err(WalletError::Decrypt);
    };

    let plaintext = key.open(body)?;
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
        };

        write(&path, &key, None, &payload).unwrap();
        let back = read(&path, &key, None).unwrap();
        assert_eq!(back.mnemonic, "phrase goes here");

        assert!(read(&path, &VaultKey::random(), None).is_err());
        assert!(!lock_info(&path).unwrap().needs_passphrase);

        let _ = fs::remove_dir_all(&dir);
    }

    fn dir_named(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "sf-store-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn sample() -> VaultPayload {
        VaultPayload {
            mnemonic: "the reference phrase".into(),
        }
    }

    #[test]
    fn a_vault_from_before_buckets_were_removed_still_opens() {

        let dir = dir_named("legacy");
        let path = dir.join("wallet.vault");
        let key = VaultKey::random();

        for old in [
            r#"{"mnemonic":"the reference phrase","buckets":[]}"#,
            r#"{"mnemonic":"the reference phrase","buckets":[{"id":"b1","name":"Rent","asset":"BTC","balanceMinor":"125000","addressCount":2}]}"#,
        ] {
            let sealed = key.seal(old.as_bytes()).unwrap();
            let mut blob = MAGIC_V2.to_vec();
            blob.push(0);
            blob.extend_from_slice(&[0u8; SALT_LEN]);
            blob.extend_from_slice(&sealed);
            fs::write(&path, &blob).unwrap();

            let back = read(&path, &key, None).unwrap();
            assert_eq!(back.mnemonic, "the reference phrase");
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_passphrase_vault_needs_both_factors() {
        let dir = dir_named("pass");
        let path = dir.join("wallet.vault");
        let keychain = VaultKey::random();

        write(&path, &keychain, Some("correct horse"), &sample()).unwrap();
        assert!(lock_info(&path).unwrap().needs_passphrase);

        assert_eq!(
            read(&path, &keychain, Some("correct horse")).unwrap().mnemonic,
            "the reference phrase"
        );

        assert!(read(&path, &keychain, Some("wrong")).is_err());

        assert!(read(&path, &keychain, None).is_err());

        assert!(read(&path, &VaultKey::random(), Some("correct horse")).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_passphrase_can_be_added_and_removed() {
        let dir = dir_named("migrate");
        let path = dir.join("wallet.vault");
        let keychain = VaultKey::random();

        write(&path, &keychain, None, &sample()).unwrap();
        let payload = read(&path, &keychain, None).unwrap();
        write(&path, &keychain, Some("secret"), &payload).unwrap();
        assert!(lock_info(&path).unwrap().needs_passphrase);

        let payload = read(&path, &keychain, Some("secret")).unwrap();
        write(&path, &keychain, None, &payload).unwrap();
        assert!(!lock_info(&path).unwrap().needs_passphrase);
        assert_eq!(read(&path, &keychain, None).unwrap().mnemonic, "the reference phrase");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_v1_files_still_open() {
        let dir = dir_named("v1");
        let path = dir.join("wallet.vault");
        let keychain = VaultKey::random();

        let json = serde_json::to_vec(&sample()).unwrap();
        let sealed = keychain.seal(&json).unwrap();
        let mut blob = MAGIC_V1.to_vec();
        blob.extend_from_slice(&sealed);
        fs::write(&path, &blob).unwrap();

        assert!(!lock_info(&path).unwrap().needs_passphrase);
        assert_eq!(read(&path, &keychain, None).unwrap().mnemonic, "the reference phrase");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_a_foreign_file() {
        let dir = dir_named("magic");
        let path = dir.join("wallet.vault");
        fs::write(&path, b"not a vault at all").unwrap();
        assert!(read(&path, &VaultKey::random(), None).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
