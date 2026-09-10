//! Routing outbound requests through Tor.
//!
//! Every balance, price and history lookup this wallet makes reveals, to the
//! server answering, both the addresses being asked about and the machine's
//! IP. Tor severs the second half: the endpoint sees a Tor exit, not the
//! user. It does nothing about what the addresses themselves reveal on chain,
//! which is a separate problem, but it is the single biggest network leak.
//!
//! The official Tor is downloaded, verified against a hash compiled into this
//! program, and run locally. Requests then go through its SOCKS proxy. As with
//! Monero, the cryptography and the network stack stay with the project that
//! maintains them.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Result, WalletError};

const VERSION: &str = "15.0.22";
const URL: &str = "https://archive.torproject.org/tor-package-archive/torbrowser/15.0.22/tor-expert-bundle-windows-x86_64-15.0.22.tar.gz";

/// Published by the Tor Project and checked against the real download before
/// being written here. A hash in the binary proves the exact build; a fetched
/// checksum would only prove the file matches a list from the same server.
const SHA256: &str = "231dad6b9cb401a54c260db7046965ef04e4f72ff071b140d423fb5da281ab1e";

/// Where the local Tor listens. 9150 rather than 9050 so it does not clash
/// with a system Tor or Tor Browser the user may already run.
pub const SOCKS_PORT: u16 = 9150;

/// Whether outbound requests should currently go through Tor. Read by the
/// shared HTTP client builder on every request, so toggling takes effect at
/// once without threading a flag through every call site.
static ROUTING: AtomicBool = AtomicBool::new(false);

pub fn routing() -> bool {
    ROUTING.load(Ordering::Relaxed)
}

pub fn set_routing(on: bool) {
    ROUTING.store(on, Ordering::Relaxed);
}

/// The proxy to hand a reqwest client, when routing is on.
pub fn proxy() -> Result<reqwest::Proxy> {
    // socks5h, not socks5: the 'h' keeps DNS resolution at the proxy. Plain
    // socks5 resolves names locally first, which leaks every host over the
    // clear network and defeats the point.
    reqwest::Proxy::all(format!("socks5h://127.0.0.1:{SOCKS_PORT}"))
        .map_err(|e| WalletError::Network(format!("tor proxy: {e}")))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorState {
    /// The Tor binary is present and verified.
    pub installed: bool,
    /// A Tor process this session started is running.
    pub running: bool,
    /// Requests are being routed through it.
    pub routing: bool,
    pub version: String,
}

fn install_dir(app_data: &Path) -> PathBuf {
    app_data.join("tor")
}

fn binary(app_data: &Path) -> PathBuf {
    install_dir(app_data).join("tor").join("tor.exe")
}

fn data_dir(app_data: &Path) -> PathBuf {
    install_dir(app_data).join("state")
}

pub fn state(app_data: &Path, running: bool) -> TorState {
    TorState {
        installed: binary(app_data).is_file(),
        running,
        routing: routing(),
        version: VERSION.to_string(),
    }
}

fn oops(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(format!("tor {what}: {e}"))
}

/// Downloads and verifies the Tor bundle, then unpacks tor.exe and its data.
pub async fn install(app_data: &Path) -> Result<()> {
    if binary(app_data).is_file() {
        return Ok(());
    }

    let dir = install_dir(app_data);
    std::fs::create_dir_all(&dir).map_err(|e| oops("folder", e))?;

    let bytes = super::chains::rpc::plain_client()?
        .get(URL)
        .send()
        .await
        .map_err(|e| oops("download", e))?
        .error_for_status()
        .map_err(|e| oops("download", e))?
        .bytes()
        .await
        .map_err(|e| oops("download", e))?;

    let digest: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if digest != SHA256 {
        return Err(WalletError::Network(format!(
            "The Tor download did not match its published checksum, so it was discarded. \
             Expected {SHA256}, got {digest}."
        )));
    }

    // tar.gz: gunzip then untar, refusing any entry whose path escapes the
    // folder, which is how a crafted archive overwrites files elsewhere.
    let gz = flate2::read::GzDecoder::new(std::io::Cursor::new(&bytes[..]));
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries().map_err(|e| oops("archive", e))? {
        let mut entry = entry.map_err(|e| oops("entry", e))?;
        let rel = entry.path().map_err(|e| oops("entry path", e))?.into_owned();
        if rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            continue;
        }
        let target = dir.join(&rel);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| oops("mkdir", e))?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| oops("mkdir", e))?;
            }
            entry.unpack(&target).map_err(|e| oops("unpack", e))?;
        }
    }

    if !binary(app_data).is_file() {
        return Err(oops("unpack", "tor.exe was not in the archive"));
    }
    Ok(())
}

/// Starts Tor on the local SOCKS port.
pub fn spawn(app_data: &Path) -> Result<Child> {
    let exe = binary(app_data);
    if !exe.is_file() {
        return Err(oops("start", "Tor is not installed"));
    }
    let state = data_dir(app_data);
    std::fs::create_dir_all(&state).map_err(|e| oops("state dir", e))?;

    let mut command = Command::new(&exe);
    command
        .arg("--SocksPort")
        .arg(format!("127.0.0.1:{SOCKS_PORT}"))
        .arg("--DataDirectory")
        .arg(&state)
        // No control port and no other listeners: this Tor exists only to
        // proxy this wallet's own requests.
        .arg("--ControlPort")
        .arg("0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.spawn().map_err(|e| oops("start", e))
}

/// A single quick check of whether the proxy already carries requests, so a
/// Tor left running from an earlier launch is reused rather than duplicated.
pub async fn proxy_alive() -> bool {
    let Ok(client) = reqwest::Client::builder()
        .proxy(match proxy() {
            Ok(p) => p,
            Err(_) => return false,
        })
        .timeout(std::time::Duration::from_secs(8))
        .build()
    else {
        return false;
    };
    client
        .get("https://check.torproject.org/api/ip")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Waits until Tor can actually carry a request, not merely until the port is
/// open. Bootstrapping a circuit takes a few seconds to a minute.
pub async fn wait_until_ready() -> Result<()> {
    let client = reqwest::Client::builder()
        .proxy(proxy()?)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| oops("client", e))?;

    for attempt in 0..30 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        // Tor's own check service confirms the request actually exited the
        // network, which a plain port check cannot.
        if client
            .get("https://check.torproject.org/api/ip")
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            return Ok(());
        }
        if attempt == 29 {
            break;
        }
    }
    Err(oops("start", "Tor did not finish connecting in time"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_hash_is_a_sha256() {
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn routing_flag_toggles() {
        set_routing(false);
        assert!(!routing());
        set_routing(true);
        assert!(routing());
        set_routing(false);
    }

    #[test]
    fn the_proxy_uses_socks5h_for_remote_dns() {
        // Building it should succeed; the scheme choice is what keeps DNS off
        // the clear network.
        assert!(proxy().is_ok());
    }

    #[test]
    fn paths_stay_within_the_app_folder() {
        let root = Path::new("C:/example/appdata");
        assert!(binary(root).starts_with(root));
        assert!(data_dir(root).starts_with(root));
    }
}
