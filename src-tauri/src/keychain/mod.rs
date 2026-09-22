use std::sync::Mutex;

#[cfg(not(target_os = "android"))]
use keyring::Entry;

use crate::crypto::aead::{VaultKey, KEY_LEN};
use crate::error::{Result, WalletError};

/// Where this app may write, set once at startup.
///
/// Only Android needs it: the desktops all have a credential store that is
/// addressed by name, so nothing there has to know a path.
#[cfg(target_os = "android")]
static APP_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Tells the Android backend which directory is ours. Called once, at setup.
#[cfg(target_os = "android")]
pub fn set_app_dir(dir: std::path::PathBuf) {
    let _ = APP_DIR.set(dir);
}

/// Android's stand-in for a credential store.
///
/// # What this is, and what it is not
///
/// Android has no Secret Service and no credential store a process can
/// address by name, so the vault key is a file in the directory Android gives
/// this application. That directory is sandboxed: no other app can read it
/// without root, and it is excluded from backup below via the manifest.
///
/// It is **weaker than the desktop backends**, and the difference is worth
/// stating plainly. On Windows and Linux the key is held by the OS and
/// released to a logged-in session; here it is a file that anyone with the
/// unlocked device, root, or a forensic image can read. Hardware-backed
/// storage would mean the Android Keystore over JNI, which this does not do
/// yet.
///
/// What closes that gap today is the optional **vault passphrase**, which is
/// already mixed with this key in `store::derive`. On a phone it stops being
/// optional in spirit: without one the key and the vault sit side by side.
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
        // Owner-only, so nothing that shares the uid can read it either.
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

/// What to call the credential store in text the user reads. Naming the real
/// one beats a vague "OS credential store" when someone has to go looking for
/// the entry, so it follows the backend the build is compiled against.
#[cfg(windows)]
pub const STORE_NAME: &str = "Windows credential store";
#[cfg(target_os = "linux")]
pub const STORE_NAME: &str = "system keyring";
#[cfg(target_os = "android")]
pub const STORE_NAME: &str = "app-private storage";

/// The account the vault key lives under.
///
/// Test builds get a per-process account instead of the real one. Without
/// that, running the suite reads and writes the credential store of whoever
/// ran it: `load_or_create` mints a key when it finds none, so a test run on
/// a machine that also *uses* this wallet could replace the real key and
/// leave the real config.dat undecryptable. A test must never be able to do
/// that.
fn account() -> String {
    #[cfg(test)]
    {
        // Per *test*, not per process. `inactivity::wipe` calls `forget`,
        // which deletes the entry — on a shared account that yanks the key
        // out from under whatever else is running in parallel, and the config
        // it had just sealed stops decrypting. The thread name is the test
        // name under the standard harness.
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

/// Reads a stored secret, or `None` when there is no entry yet.
///
/// The two backends report "not there" differently — one as an error variant,
/// the other as a missing file — so both are normalised here and every caller
/// below reads the same shape.
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
            // Already gone is the state we wanted.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(WalletError::Keychain(e.to_string())),
        }
    }
}

/// Turns stored bytes into a key, refusing anything the wrong length.
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

/// The vault key lives in the OS credential store, not in anything the user
/// types. That means the encrypted vault on disk is useless to someone who
/// copies the file off this machine without also holding the OS account.
///
/// Called per vault read or write rather than cached, so the key sits in
/// process memory for as short a window as possible.
/// Serialises the read-then-maybe-create below.
///
/// The credential store has no compare-and-swap, so two threads arriving at
/// once could both see no entry, both mint a key, and both write — leaving
/// whatever the loser sealed unreadable under the winner's key. Rare, but the
/// damage is a vault that will not open, so the window is closed rather than
/// tolerated.
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

/// Reads the key only if one already exists. Unlocking must not silently mint
/// a new key, since that would decrypt nothing and look like vault damage.
pub fn load() -> Result<VaultKey> {
    match get_secret(&account())? {
        Some(bytes) => key_from(&bytes),
        None => Err(WalletError::NoVault),
    }
}

/// Removes the vault key from the OS credential store.
///
/// Without the key the encrypted vault file is unreadable forever, so this is
/// only ever called alongside deleting that file, and only after the user has
/// confirmed in writing.
pub fn forget() -> Result<()> {
    delete_secret(&account())
}

const MONERO_ACCOUNT: &str = "monero-wallet-password";

/// Password for the Monero wallet file, generated once and kept in the OS
/// credential store.
///
/// The user never sees or types it. Losing it costs nothing permanent: the
/// account can be recreated from the seed at any time.
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

    /// Proves the credential store this build is compiled against actually
    /// answers: the Windows credential store, or the Secret Service on Linux.
    /// Ignored because it needs a real desktop session — on Linux a running
    /// gnome-keyring or KWallet with an unlocked collection, which CI has not
    /// got. Uses its own account so it never touches the real vault key.
    ///
    /// `cargo test keychain_round_trips -- --ignored --nocapture`
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
