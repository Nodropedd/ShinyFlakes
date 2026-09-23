//! App settings, persisted to disk.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Result, WalletError};

const MAGIC: &[u8; 4] = b"SFC\x01";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepUp {

    #[serde(default)]
    pub dormant_days: u32,

    #[serde(default)]
    pub large_send: bool,
}

pub const DEFAULT_DORMANT_DAYS: u32 = 15;

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

    #[serde(default)]
    pub totp_secret: String,

    #[serde(default)]
    pub step_up: StepUp,

    #[serde(default)]
    pub stay_signed_in: bool,

    #[serde(default)]
    pub sign_in_paused: bool,

    #[serde(default)]
    pub sol_exodus_for: Option<String>,
}

pub fn is_unreadable(app_data: &Path) -> bool {
    matches!(load(app_data), Err(WalletError::SettingsUnreadable))
}

pub fn reset(app_data: &Path) -> Result<()> {
    let file = path(app_data);
    if file.exists() {

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
            sol_exodus_for: Some("wallet".into()),
        };
        save(&d, &cfg).unwrap();

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
