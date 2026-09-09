use keyring::Entry;

use crate::crypto::aead::{VaultKey, KEY_LEN};
use crate::error::{Result, WalletError};

const SERVICE: &str = "ShinyFlakes";
const ACCOUNT: &str = "vault-key";

fn entry() -> Result<Entry> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| WalletError::Keychain(e.to_string()))
}

/// The vault key lives in the OS credential store, not in anything the user
/// types. That means the encrypted vault on disk is useless to someone who
/// copies the file off this machine without also holding the OS account.
///
/// Called per vault read or write rather than cached, so the key sits in
/// process memory for as short a window as possible.
pub fn load_or_create() -> Result<VaultKey> {
    let entry = entry()?;
    match entry.get_secret() {
        Ok(bytes) if bytes.len() == KEY_LEN => {
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&bytes);
            Ok(VaultKey::from_bytes(key))
        }
        Ok(_) => Err(WalletError::Keychain(
            "the stored vault key is the wrong length".into(),
        )),
        Err(keyring::Error::NoEntry) => {
            let key = VaultKey::random();
            entry
                .set_secret(key.expose())
                .map_err(|e| WalletError::Keychain(e.to_string()))?;
            Ok(key)
        }
        Err(e) => Err(WalletError::Keychain(e.to_string())),
    }
}

/// Reads the key only if one already exists. Unlocking must not silently mint
/// a new key, since that would decrypt nothing and look like vault damage.
pub fn load() -> Result<VaultKey> {
    let entry = entry()?;
    match entry.get_secret() {
        Ok(bytes) if bytes.len() == KEY_LEN => {
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&bytes);
            Ok(VaultKey::from_bytes(key))
        }
        Ok(_) => Err(WalletError::Keychain(
            "the stored vault key is the wrong length".into(),
        )),
        Err(keyring::Error::NoEntry) => Err(WalletError::NoVault),
        Err(e) => Err(WalletError::Keychain(e.to_string())),
    }
}

/// Removes the vault key from the OS credential store.
///
/// Without the key the encrypted vault file is unreadable forever, so this is
/// only ever called alongside deleting that file, and only after the user has
/// confirmed in writing.
pub fn forget() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(()),
        // Already gone is the state we wanted.
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(WalletError::Keychain(e.to_string())),
    }
}

const MONERO_ACCOUNT: &str = "monero-wallet-password";

/// Password for the Monero wallet file, generated once and kept in the OS
/// credential store.
///
/// The user never sees or types it. Losing it costs nothing permanent: the
/// account can be recreated from the seed at any time.
pub fn monero_password() -> Result<String> {
    let entry = Entry::new(SERVICE, MONERO_ACCOUNT)
        .map_err(|e| WalletError::Keychain(e.to_string()))?;

    match entry.get_password() {
        Ok(existing) if !existing.is_empty() => Ok(existing),
        Ok(_) | Err(keyring::Error::NoEntry) => {
            use rand::RngCore;
            let mut bytes = [0u8; 24];
            rand::rngs::OsRng.fill_bytes(&mut bytes);
            let password: String = bytes.iter().map(|b| format!("{b:02x}")).collect();

            entry
                .set_password(&password)
                .map_err(|e| WalletError::Keychain(e.to_string()))?;
            Ok(password)
        }
        Err(e) => Err(WalletError::Keychain(e.to_string())),
    }
}
