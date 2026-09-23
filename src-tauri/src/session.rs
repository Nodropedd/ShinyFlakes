//! Unlocked session state.

use std::path::PathBuf;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::crypto::seed::SEED_LEN;

pub struct Unlocked {

    pub seed: Zeroizing<[u8; SEED_LEN]>,

    #[allow(dead_code)]
    pub mnemonic: Zeroizing<String>,
}

pub struct AppState {
    pub vault_path: PathBuf,
    pub unlocked: Mutex<Option<Unlocked>>,

    pub data_dir: PathBuf,

    pub monero: Mutex<Option<std::process::Child>>,

    pub tor: Mutex<Option<std::process::Child>>,

    pub two_factor: Mutex<TwoFactor>,
}

#[derive(Default)]
pub struct TwoFactor {

    pub pending_secret: Option<String>,

    pub pass_expiry: Option<i64>,

    pub dormant_pending: bool,

    pub wrong: u32,

    pub locked_until: Option<i64>,
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

    pub fn stop_monero(&self) {
        if let Ok(mut guard) = self.monero.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

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

    pub fn wipe(&self) {
        if let Ok(mut guard) = self.unlocked.lock() {
            *guard = None;
        }
        crate::chains::set_sol_exodus(false);

        if let Ok(mut tf) = self.two_factor.lock() {
            *tf = TwoFactor::default();
        }

        self.stop_monero();
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }
}
