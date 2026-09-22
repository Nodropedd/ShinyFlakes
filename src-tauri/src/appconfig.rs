//! Local settings that hold a secret: the two-factor secret, and when the
//! wallet should ask for it.
//!
//! Kept in their own encrypted file rather than in the vault, sealed with the
//! keychain key alone. That means they can be read and written whenever the
//! app runs, without the seed or the vault passphrase, which is what the
//! two-factor flow needs. The TOTP secret inside is confidential but far less
//! catastrophic than the seed, so keychain-key encryption at rest, the same
//! the vault had before passphrases, is the right level.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Result, WalletError};

const MAGIC: &[u8; 4] = b"SFC\x01";

/// When the wallet should stop and ask for a second factor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepUp {
    /// Days of not being opened before the next unlock has to step up. Zero
    /// turns the dormancy trigger off.
    #[serde(default)]
    pub dormant_days: u32,
    /// Guard a send worth more than [`LARGE_SEND_USD`] that is also at least
    /// [`LARGE_SEND_SHARE`] of everything the wallet holds.
    #[serde(default)]
    pub large_send: bool,
}

/// Days of silence before a step-up is demanded, unless the user changes it.
pub const DEFAULT_DORMANT_DAYS: u32 = 15;

/// A send has to clear both of these to count as large: enough money to care
/// about, and enough of the wallet that it is not routine.
pub const LARGE_SEND_USD: f64 = 50.0;
pub const LARGE_SEND_SHARE: f64 = 0.10;

impl Default for StepUp {
    fn default() -> Self {
        StepUp {
            dormant_days: DEFAULT_DORMANT_DAYS,
            large_send: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub two_factor: bool,
    /// The base32 TOTP secret shared with the authenticator app. Present only
    /// while two-factor is on.
    #[serde(default)]
    pub totp_secret: String,
    /// When the wallet demands a second factor.
    #[serde(default)]
    pub step_up: StepUp,
    /// Open straight into the wallet when the app starts, instead of asking
    /// for the seed phrase. Off until the user turns it on.
    #[serde(default)]
    pub stay_signed_in: bool,
    /// Set by Lock and cleared by the next unlock with the phrase, so that
    /// pressing Lock still means the next start asks for it.
    #[serde(default)]
    pub sign_in_paused: bool,
}

/// Whether the settings file exists but cannot be read back.
pub fn is_unreadable(app_data: &Path) -> bool {
    matches!(load(app_data), Err(WalletError::SettingsUnreadable))
}

/// Puts the unreadable file aside and starts fresh.
///
/// Deliberately never automatic. Settings hold whether two-factor is on, so a
/// silent reset would be a way to switch the gate off by corrupting a file —
/// exactly the move an attacker would want. The user has to ask.
///
/// Nothing irreplaceable is lost: the seed is not in here. What goes is the
/// step-up choices and the TOTP secret, so the authenticator has to be paired
/// again.
pub fn reset(app_data: &Path) -> Result<()> {
    let file = path(app_data);
    if file.exists() {
        // Kept rather than deleted, in case the key that opens it turns up.
        let aside = file.with_extension("dat.unreadable");
        let _ = std::fs::rename(&file, &aside);
    }
    save(app_data, &AppConfig::default())
}

fn path(app_data: &Path) -> PathBuf {
    app_data.join("config.dat")
}

pub fn load(app_data: &Path) -> Result<AppConfig> {
    let blob = match std::fs::read(path(app_data)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(AppConfig::default()),
        Err(e) => return Err(WalletError::Storage(e.to_string())),
    };
    // Every failure from here down is the *settings* file, not the vault.
    // Saying "the vault could not be decrypted" when the seed is perfectly
    // safe is alarming and wrong, so this reports itself as what it is and
    // stays recoverable — see `reset`.
    if blob.len() <= MAGIC.len() || &blob[..MAGIC.len()] != MAGIC {
        return Err(WalletError::SettingsUnreadable);
    }
    let key = crate::keychain::load_or_create()?;
    let plain = key
        .open(&blob[MAGIC.len()..])
        .map_err(|_| WalletError::SettingsUnreadable)?;
    serde_json::from_slice(&plain).map_err(|_| WalletError::SettingsUnreadable)
}

pub fn save(app_data: &Path, config: &AppConfig) -> Result<()> {
    let key = crate::keychain::load_or_create()?;
    let json = Zeroizing::new(
        serde_json::to_vec(config).map_err(|e| WalletError::Storage(e.to_string()))?,
    );
    let sealed = key.seal(&json)?;

    if let Some(parent) = path(app_data).parent() {
        std::fs::create_dir_all(parent).map_err(|e| WalletError::Storage(e.to_string()))?;
    }
    let mut out = Vec::with_capacity(MAGIC.len() + sealed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&sealed);

    let temp = path(app_data).with_extension("dat.tmp");
    std::fs::write(&temp, &out).map_err(|e| WalletError::Storage(e.to_string()))?;
    std::fs::rename(&temp, path(app_data)).map_err(|e| WalletError::Storage(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_from_before_staying_signed_in_leave_it_off() {
        // What earlier builds wrote. Upgrading must never sign anyone in on
        // its own: the owner has to turn this on.
        let old = r#"{"twoFactor":true,"totpSecret":"JBSWY3DPEHPK3PXP","stepUp":{"dormantDays":15,"largeSend":true}}"#;
        let cfg: AppConfig = serde_json::from_str(old).unwrap();
        assert!(!cfg.stay_signed_in);
        assert!(!cfg.sign_in_paused);
        assert!(cfg.two_factor);
        assert!(!AppConfig::default().stay_signed_in);
    }

    fn dir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "sf-cfg-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_missing_file_reads_as_defaults() {
        let d = dir();
        let cfg = load(&d).unwrap();
        assert!(!cfg.two_factor);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn config_round_trips_encrypted() {
        let d = dir();
        let cfg = AppConfig {
            two_factor: true,
            totp_secret: "GEZDGNBVGY3TQOJQ".into(),
            step_up: StepUp::default(),
            stay_signed_in: true,
            sign_in_paused: false,
        };
        save(&d, &cfg).unwrap();

        // The secret may not sit in the file in the clear.
        let raw = std::fs::read(path(&d)).unwrap();
        assert!(!raw
            .windows("GEZDGNBVGY3TQOJQ".len())
            .any(|w| w == b"GEZDGNBVGY3TQOJQ"));

        let back = load(&d).unwrap();
        assert!(back.two_factor);
        assert_eq!(back.totp_secret, "GEZDGNBVGY3TQOJQ");
        assert!(back.stay_signed_in);
        assert!(!back.sign_in_paused);
        let _ = std::fs::remove_dir_all(&d);
    }
}

