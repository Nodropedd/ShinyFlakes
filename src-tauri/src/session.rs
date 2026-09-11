use std::path::PathBuf;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::crypto::seed::SEED_LEN;
use crate::store::Bucket;

/// What exists only while the wallet is unlocked. Dropping this zeroes the
/// seed, which is what lock, logout, and the idle timeout all do.
pub struct Unlocked {
    /// Every per-chain address and key derives from this.
    pub seed: Zeroizing<[u8; SEED_LEN]>,
    /// Kept so the seed can be re-shown for backup behind a confirmation.
    #[allow(dead_code)]
    pub mnemonic: Zeroizing<String>,
    pub buckets: Vec<Bucket>,
}

pub struct AppState {
    pub vault_path: PathBuf,
    pub unlocked: Mutex<Option<Unlocked>>,
    /// Folder this app owns, where Monero is installed if the user asks.
    pub data_dir: PathBuf,
    /// Monero's wallet daemon while it is running. Stopped on lock, on logout
    /// and on exit, so it never outlives the session that started it.
    pub monero: Mutex<Option<std::process::Child>>,
    /// The Tor process while it is running. Killed on exit, like the daemon.
    /// It is not tied to lock, since browsing balances is not privileged and
    /// tearing Tor down on every lock would make it slow to come back.
    pub tor: Mutex<Option<std::process::Child>>,
    /// The two-factor code in flight and, once one is passed, when that pass
    /// runs out. Both are cleared on lock along with the keys.
    pub two_factor: Mutex<TwoFactor>,
}

#[derive(Default)]
pub struct TwoFactor {
    pub pending: Option<crate::twofa::Pending>,
    pub pass_expiry: Option<i64>,
}

impl AppState {
    pub fn new(vault_path: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            vault_path,
            unlocked: Mutex::new(None),
            data_dir,
            monero: Mutex::new(None),
            tor: Mutex::new(None),
            two_factor: Mutex::new(TwoFactor::default()),
        }
    }

    /// Stops the Monero daemon, if this session started one.
    pub fn stop_monero(&self) {
        if let Ok(mut guard) = self.monero.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// Stops the Tor process, if this session started one.
    pub fn stop_tor(&self) {
        if let Ok(mut guard) = self.tor.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    pub fn tor_running(&self) -> bool {
        self.tor.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn monero_running(&self) -> bool {
        self.monero
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Drops derived key material. The encrypted vault on disk is untouched:
    /// locking must never be able to destroy a wallet.
    pub fn wipe(&self) {
        if let Ok(mut guard) = self.unlocked.lock() {
            *guard = None;
        }
        // A pending code or a live two-factor pass must not survive a lock,
        // or it could gate a reveal for whoever unlocks next.
        if let Ok(mut tf) = self.two_factor.lock() {
            *tf = TwoFactor::default();
        }
        // That daemon holds an open Monero wallet, so locking has to close it
        // too. Otherwise the money stays reachable after the keys are gone.
        self.stop_monero();
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }
}
