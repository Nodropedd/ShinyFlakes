//! One-button Monero setup.
//!
//! Downloads Monero's official wallet daemon, verifies it, and starts it
//! against this wallet's own account. The account is created by asking the
//! daemon over localhost, so the spend key goes straight from memory into
//! Monero's own process and is never written to a file or shown on screen.
//!
//! The download is pinned to one release and one hash. A fetched checksum
//! list would only prove the list and the file came from the same place; a
//! hash compiled into the binary proves it is the exact build this code was
//! written against.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Result, WalletError};

const VERSION: &str = "v0.18.5.1";
const ARCHIVE: &str = "monero-win-x64-v0.18.5.1.zip";
const URL: &str = "https://downloads.getmonero.org/cli/monero-win-x64-v0.18.5.1.zip";

/// Published by the Monero project at getmonero.org/downloads/hashes.txt and
/// checked against the real download before being written here.
const SHA256: &str = "cf2ae8273977697d9ef2031c7337b781e6e5936578f602444b2990a173a2437d";

/// Folder inside the archive.
const INNER: &str = "monero-x86_64-w64-mingw32-v0.18.5.1";

/// A public node to read the chain from until the user runs their own.
///
/// It never sees the keys and cannot spend anything, but it does see this
/// machine's address and which parts of the chain are requested. That is
/// stated in the interface rather than buried here.
pub const DEFAULT_DAEMON: &str = "xmr-node.cakewallet.com:18081";

pub const RPC_PORT: u16 = 18082;

/// Roughly a month of blocks. A freshly derived account cannot hold anything
/// older than the day it was first used, so scanning from further back only
/// wastes time; a month of slack covers a wallet set up a while ago.
const RESTORE_SLACK_BLOCKS: u64 = 21_600;

fn oops(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(format!("monero setup: {what}: {e}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {
    /// The daemon binary is present and verified.
    pub installed: bool,
    /// A wallet file for this account exists.
    pub wallet_exists: bool,
    /// The bridge is running right now.
    pub running: bool,
    pub version: String,
}

pub fn install_dir(app_data: &Path) -> PathBuf {
    app_data.join("monero")
}

fn binary(app_data: &Path) -> PathBuf {
    install_dir(app_data)
        .join(INNER)
        .join("monero-wallet-rpc.exe")
}

fn wallet_dir(app_data: &Path) -> PathBuf {
    app_data.join("monero-wallet")
}

fn wallet_path(app_data: &Path) -> PathBuf {
    wallet_dir(app_data).join("account")
}

pub fn state(app_data: &Path, running: bool) -> SetupState {
    SetupState {
        installed: binary(app_data).is_file(),
        wallet_exists: wallet_path(app_data).is_file(),
        running,
        version: VERSION.to_string(),
    }
}

// ---------- install ----------

/// Downloads the official release, checks it against the pinned hash, and
/// unpacks it. Refuses to unpack anything whose hash does not match.
pub async fn install(app_data: &Path) -> Result<()> {
    if binary(app_data).is_file() {
        return Ok(());
    }

    let dir = install_dir(app_data);
    std::fs::create_dir_all(&dir).map_err(|e| oops("creating the folder", e))?;

    let bytes = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .build()
        .map_err(|e| oops("http client", e))?
        .get(URL)
        .send()
        .await
        .map_err(|e| oops("download", e))?
        .error_for_status()
        .map_err(|e| oops("download", e))?
        .bytes()
        .await
        .map_err(|e| oops("download", e))?;

    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    if digest != SHA256 {
        return Err(WalletError::Network(format!(
            "The Monero download did not match its published checksum, so it was discarded. \
             Expected {SHA256}, got {digest}."
        )));
    }

    let archive = dir.join(ARCHIVE);
    std::fs::write(&archive, &bytes).map_err(|e| oops("saving the archive", e))?;

    let file = std::fs::File::open(&archive).map_err(|e| oops("opening the archive", e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| oops("reading the archive", e))?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| oops("reading an entry", e))?;

        // enclosed_name refuses paths that would escape the folder, which is
        // how a malicious archive overwrites files elsewhere on the disk.
        let Some(relative) = entry.enclosed_name() else {
            continue;
        };
        let target = dir.join(relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| oops("creating a folder", e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| oops("creating a folder", e))?;
        }

        let mut out = std::fs::File::create(&target).map_err(|e| oops("writing a file", e))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| oops("writing a file", e))?;
    }

    let _ = std::fs::remove_file(&archive);

    if !binary(app_data).is_file() {
        return Err(oops("unpacking", "the wallet daemon was not in the archive"));
    }
    Ok(())
}

// ---------- running ----------

/// Starts the daemon with no wallet open, so it can be asked to make one.
pub fn spawn(app_data: &Path, daemon: &str) -> Result<Child> {
    let exe = binary(app_data);
    if !exe.is_file() {
        return Err(oops("start", "the Monero wallet daemon is not installed"));
    }

    let dir = wallet_dir(app_data);
    std::fs::create_dir_all(&dir).map_err(|e| oops("creating the wallet folder", e))?;

    let mut command = Command::new(&exe);
    command
        .arg("--wallet-dir")
        .arg(&dir)
        .arg("--rpc-bind-port")
        .arg(RPC_PORT.to_string())
        .arg("--rpc-bind-ip")
        .arg("127.0.0.1")
        .arg("--disable-rpc-login")
        .arg("--daemon-address")
        .arg(daemon)
        // Deliberately no --trusted-daemon: the node above is someone
        // else's, so the wallet does the sensitive work itself.
        .arg("--log-level")
        .arg("0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window: this is a background helper, not something the
        // user should have to keep on screen.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.spawn().map_err(|e| oops("start", e))
}

fn pid_file(app_data: &Path) -> PathBuf {
    install_dir(app_data).join("daemon.pid")
}

/// Ends a daemon left behind by an earlier run.
///
/// The app restarts whenever it is rebuilt, which loses the handle to any
/// child it started. That orphan keeps the wallet file open, and the next
/// attempt then fails with "opened by another wallet program". The process id
/// is recorded on disk so it can be found again across restarts.
fn kill_stale(app_data: &Path) {
    let path = pid_file(app_data);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };

    if let Ok(pid) = text.trim().parse::<u32>() {
        // By id rather than by name, so a Monero wallet the user runs
        // themselves is never touched.
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        {
            let _ = Command::new("kill")
                .args(["-9", &pid.to_string()])
                .status();
        }
    }

    let _ = std::fs::remove_file(&path);
}

fn record_pid(app_data: &Path, pid: u32) {
    let _ = std::fs::write(pid_file(app_data), pid.to_string());
}

/// Ends whichever daemon is running for this wallet, including one started
/// by an earlier run of the app.
pub fn stop_any(app_data: &Path) {
    kill_stale(app_data);
}

/// Gets a daemon running, one way or another.
///
/// If something is already answering on the port it is used as it is, rather
/// than starting a second one that could not bind anyway. Otherwise any
/// orphan is cleaned up and a fresh daemon is started.
pub async fn ensure_running(
    app_data: &Path,
    daemon: &str,
    endpoint: &str,
) -> Result<Option<Child>> {
    if super::xmr_rpc::ping(endpoint).await.is_ok() {
        return Ok(None);
    }

    kill_stale(app_data);

    let child = spawn(app_data, daemon)?;
    record_pid(app_data, child.id());
    wait_until_ready(endpoint).await?;
    Ok(Some(child))
}

/// Waits for the daemon to answer, since it takes a moment to bind.
pub async fn wait_until_ready(endpoint: &str) -> Result<()> {
    for attempt in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if super::xmr_rpc::ping(endpoint).await.is_ok() {
            return Ok(());
        }
        if attempt == 39 {
            break;
        }
    }
    Err(oops("start", "the wallet daemon did not come up in time"))
}

// ---------- the account ----------

/// Creates the wallet file from this account's keys, if it is not there.
///
/// The keys travel over the loopback RPC into Monero's own process. They are
/// never written to disk by this program and never shown.
#[allow(clippy::too_many_arguments)]
pub async fn ensure_wallet(
    app_data: &Path,
    endpoint: &str,
    daemon: &str,
    address: &str,
    spend_key: &str,
    view_key: &str,
    password: &str,
) -> Result<()> {
    if wallet_path(app_data).is_file() {
        match super::xmr_rpc::open_wallet(endpoint, "account", password).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                // The daemon may already have it open, in which case asking
                // again is refused but everything works. Balance is the
                // honest test of that.
                if super::xmr_rpc::balance(endpoint).await.is_ok() {
                    return Ok(());
                }
                return Err(WalletError::Network(format!(
                    "{e}\n\nSomething else is holding this Monero wallet open. Close any \
                     Monero wallet program that is running, then try again."
                )));
            }
        }
    }

    // Asked of the node directly: the wallet daemon cannot answer this
    // until a wallet is open, and there is not one yet.
    let height = daemon_height(daemon)
        .await
        .unwrap_or(0)
        .saturating_sub(RESTORE_SLACK_BLOCKS);

    super::xmr_rpc::generate_from_keys(
        endpoint, "account", address, spend_key, view_key, password, height,
    )
    .await?;

    // Creating the wallet also opens it and takes a lock on the keys file.
    // Asking to open it again makes the daemon collide with its own lock,
    // which Windows reports as a sharing violation and Monero relays as
    // "opened by another wallet program". So there is nothing left to do.
    Ok(())
}

/// Current chain tip according to the node, used to pick a restore height.
///
/// This is a plain read from a public node and involves no keys, so it is
/// not held to the loopback rule the wallet daemon is.
async fn daemon_height(daemon: &str) -> Result<u64> {
    #[derive(serde::Deserialize)]
    struct Info {
        height: u64,
    }

    let info: Info = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| oops("http client", e))?
        .get(format!("http://{daemon}/get_info"))
        .send()
        .await
        .map_err(|e| oops("node height", e))?
        .error_for_status()
        .map_err(|e| oops("node height", e))?
        .json()
        .await
        .map_err(|e| oops("node height", e))?;

    Ok(info.height)
}

/// A random password for the wallet file, kept in the OS credential store
/// beside the vault key so the user never has to know or type it.
pub fn wallet_password() -> Result<String> {
    crate::keychain::monero_password()
}

/// Writes a short note next to the wallet explaining what it is, for anyone
/// who finds the folder later and wonders.
pub fn write_readme(app_data: &Path) {
    let note = format!(
        "This folder holds a Monero wallet created by ShinyFlakes {VERSION}.\n\
         \n\
         It was restored from the keys this wallet derives from your seed\n\
         phrase. Its password is stored in the Windows credential store.\n\
         \n\
         Deleting it loses nothing permanently: it can be recreated from the\n\
         same seed, using the keys shown under Settings, Monero keys.\n"
    );
    let path = wallet_dir(app_data).join("README.txt");
    if let Ok(mut file) = std::fs::File::create(path) {
        let _ = file.write_all(note.as_bytes());
    }
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
    fn paths_stay_inside_the_app_folder() {
        let root = Path::new("C:/example/appdata");
        assert!(binary(root).starts_with(root));
        assert!(wallet_path(root).starts_with(root));
        assert!(install_dir(root).starts_with(root));
    }

    #[test]
    fn state_reports_nothing_installed_for_an_empty_folder() {
        let root = Path::new("C:/definitely/not/here");
        let s = state(root, false);
        assert!(!s.installed);
        assert!(!s.wallet_exists);
        assert!(!s.running);
        assert_eq!(s.version, VERSION);
    }
}
