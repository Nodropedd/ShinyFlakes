//! Vault key storage.

use std::sync::Mutex;

#[cfg(not(target_os = "android"))]
use keyring::Entry;

use crate::crypto::aead::{VaultKey, KEY_LEN};
use crate::error::{Result, WalletError};

#[cfg(target_os = "android")]
static APP_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

#[cfg(target_os = "android")]
pub fn set_app_dir(dir: std::path::PathBuf) {
    let _ = APP_DIR.set(dir);
}

#[cfg(target_os = "android")]
mod android_store {
    use super::*;

    fn key_file(account: &str) -> Result<std::path::PathBuf> {
        let dir = APP_DIR
            .get()
            .ok_or_else(|| WalletError::Keychain("the app directory is not set yet".into()))?;
        std::fs::create_dir_all(dir).map_err(|e| WalletError::Keychain(e.to_string()))?;
        Ok(dir.join(format!("{account}.key")))
    }

    pub fn get(account: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(key_file(account)?) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(WalletError::Keychain(e.to_string())),
        }
    }

    pub fn set(account: &str, secret: &[u8]) -> Result<()> {
        let path = key_file(account)?;
        std::fs::write(&path, secret).map_err(|e| WalletError::Keychain(e.to_string()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn delete(account: &str) -> Result<()> {
        match std::fs::remove_file(key_file(account)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(WalletError::Keychain(e.to_string())),
        }
    }
}

const SERVICE: &str = "ShinyFlakes";
const ACCOUNT: &str = "vault-key";

#[cfg(windows)]
pub const STORE_NAME: &str = "Windows credential store";
#[cfg(target_os = "linux")]
pub const STORE_NAME: &str = "system keyring";
#[cfg(target_os = "android")]
pub const STORE_NAME: &str = "app-private storage";

fn account() -> String {
    #[cfg(test)]
    {

        let thread = std::thread::current();
        format!(
            "test-{}-{}-{}",
            ACCOUNT,
            std::process::id(),
            thread.name().unwrap_or("unnamed")
        )
    }
    #[cfg(not(test))]
    {
        ACCOUNT.to_string()
    }
}

#[cfg(not(target_os = "android"))]
fn entry_for(account: &str) -> Result<Entry> {
    Entry::new(SERVICE, account).map_err(|e| WalletError::Keychain(e.to_string()))
}

fn get_secret(account: &str) -> Result<Option<Vec<u8>>> {
    #[cfg(target_os = "android")]
    {
        android_store::get(account)
    }
    #[cfg(not(target_os = "android"))]
    {
        match entry_for(account)?.get_secret() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(WalletError::Keychain(e.to_string())),
        }
    }
}

fn set_secret(account: &str, secret: &[u8]) -> Result<()> {
    #[cfg(target_os = "android")]
    {
        android_store::set(account, secret)
    }
    #[cfg(not(target_os = "android"))]
    {
        entry_for(account)?
            .set_secret(secret)
            .map_err(|e| WalletError::Keychain(e.to_string()))
    }
}

fn delete_secret(account: &str) -> Result<()> {
    #[cfg(target_os = "android")]
    {
        android_store::delete(account)
    }
    #[cfg(not(target_os = "android"))]
    {
        match entry_for(account)?.delete_credential() {
            Ok(()) => Ok(()),

            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(WalletError::Keychain(e.to_string())),
        }
    }
}

fn key_from(bytes: &[u8]) -> Result<VaultKey> {
    if bytes.len() != KEY_LEN {
        return Err(WalletError::Keychain(
            "the stored vault key is the wrong length".into(),
        ));
    }
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(bytes);
    Ok(VaultKey::from_bytes(key))
}

static MINTING: Mutex<()> = Mutex::new(());

pub fn load_or_create() -> Result<VaultKey> {
    let _guard = MINTING
        .lock()
        .map_err(|_| WalletError::Keychain("key lock poisoned".into()))?;
    let account = account();
    match get_secret(&account)? {
        Some(bytes) => key_from(&bytes),
        None => {
            let key = VaultKey::random();
            set_secret(&account, key.expose())?;
            Ok(key)
        }
    }
}

pub fn load() -> Result<VaultKey> {
    match get_secret(&account())? {
        Some(bytes) => key_from(&bytes),
        None => Err(WalletError::NoVault),
    }
}

pub fn forget() -> Result<()> {
    delete_secret(&account())
}

const MONERO_ACCOUNT: &str = "monero-wallet-password";

pub fn monero_password() -> Result<String> {
    if let Some(bytes) = get_secret(MONERO_ACCOUNT)? {
        if let Ok(existing) = String::from_utf8(bytes) {
            if !existing.is_empty() {
                return Ok(existing);
            }
        }
    }

    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let password: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    set_secret(MONERO_ACCOUNT, password.as_bytes())?;
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn keychain_round_trips() {
        let secret = [7u8; KEY_LEN];

        set_secret("test-round-trip", &secret).expect("write to the store");
        let read = get_secret("test-round-trip")
            .expect("read it back")
            .expect("an entry should exist");
        assert_eq!(read, secret, "the store returned different bytes");

        delete_secret("test-round-trip").expect("clean up");
        assert!(get_secret("test-round-trip").expect("query").is_none());

        println!("credential store reachable: {STORE_NAME}");
    }
}
