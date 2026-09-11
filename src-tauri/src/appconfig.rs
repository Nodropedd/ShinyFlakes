//! Local settings that hold a secret: the SMTP credentials, and whether
//! two-factor is on.
//!
//! Kept in their own encrypted file rather than in the vault, sealed with the
//! keychain key alone. That means they can be read and written whenever the
//! app runs, without the seed or the vault passphrase, which is what the email
//! and two-factor flows need. The app password inside is confidential but far
//! less catastrophic than the seed, so keychain-key encryption at rest, the
//! same the vault had before passphrases, is the right level.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Result, WalletError};

const MAGIC: &[u8; 4] = b"SFC\x01";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Smtp {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// App-specific password. Never leaves this machine except to the user's
    /// own mail server over TLS.
    pub password: String,
    /// The address mail is sent from and, being the user's own mailbox, the
    /// address notifications and codes are sent to.
    pub from: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Swap {
    /// The Trocador API key. Free to obtain, and the identity commission is
    /// attributed to. Confidential but far less catastrophic than the seed.
    pub api_key: String,
    /// A percentage added on top of the rate, paid to that key's Trocador
    /// account. Zero means none. This is how the wallet's owner earns on swaps.
    pub markup: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub smtp: Option<Smtp>,
    pub two_factor: bool,
    pub swap: Option<Swap>,
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
        return Err(WalletError::Decrypt);
    }
    let key = crate::keychain::load_or_create()?;
    let plain = key.open(&blob[MAGIC.len()..])?;
    serde_json::from_slice(&plain).map_err(|_| WalletError::Decrypt)
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
        assert!(cfg.smtp.is_none());
        assert!(!cfg.two_factor);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn config_round_trips_encrypted() {
        let d = dir();
        let cfg = AppConfig {
            smtp: Some(Smtp {
                host: "smtp.example.com".into(),
                port: 587,
                username: "me@example.com".into(),
                password: "app-secret".into(),
                from: "me@example.com".into(),
            }),
            two_factor: true,
            swap: Some(Swap {
                api_key: "trocador-secret".into(),
                markup: 0.5,
            }),
        };
        save(&d, &cfg).unwrap();

        // Neither secret may sit in the file in the clear.
        let raw = std::fs::read(path(&d)).unwrap();
        assert!(!raw.windows(10).any(|w| w == b"app-secret"));
        assert!(!raw
            .windows("trocador-secret".len())
            .any(|w| w == b"trocador-secret"));

        let back = load(&d).unwrap();
        assert!(back.two_factor);
        assert_eq!(back.smtp.unwrap().host, "smtp.example.com");
        let swap = back.swap.unwrap();
        assert_eq!(swap.api_key, "trocador-secret");
        assert_eq!(swap.markup, 0.5);
        let _ = std::fs::remove_dir_all(&d);
    }
}
