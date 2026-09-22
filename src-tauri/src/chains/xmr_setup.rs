//! Monero daemon lifecycle.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde::Serialize;
#[cfg(not(target_os = "android"))]
use sha2::{Digest, Sha256};

use crate::error::{Result, WalletError};

const VERSION: &str = "v0.18.5.1";

#[cfg(windows)]
const ARCHIVE: &str = "monero-win-x64-v0.18.5.1.zip";
#[cfg(windows)]
const URL: &str = "https://downloads.getmonero.org/cli/monero-win-x64-v0.18.5.1.zip";
#[cfg(target_os = "linux")]
const URL: &str = "https://downloads.getmonero.org/cli/monero-linux-x64-v0.18.5.1.tar.bz2";

#[cfg(windows)]
const SHA256: &str = "cf2ae8273977697d9ef2031c7337b781e6e5936578f602444b2990a173a2437d";
#[cfg(target_os = "linux")]
const SHA256: &str = "22a7dda7b0cb699fdd6b7674c3b4a4465b337cc98a54983523b759e1e7cc9958";

#[cfg(windows)]
const INNER: &str = "monero-x86_64-w64-mingw32-v0.18.5.1";
#[cfg(target_os = "linux")]
const INNER: &str = "monero-x86_64-linux-gnu-v0.18.5.1";

#[cfg(windows)]
const RPC_BIN: &str = "monero-wallet-rpc.exe";
#[cfg(all(unix, not(target_os = "android")))]
const RPC_BIN: &str = "monero-wallet-rpc";

#[cfg(target_os = "android")]
const RPC_BIN: &str = "libmonero_wallet_rpc.so";

pub const DEFAULT_DAEMON: &str = "xmr-node.cakewallet.com:18081";

pub const RPC_PORT: u16 = 18082;

const RESTORE_SLACK_BLOCKS: u64 = 21_600;

fn oops(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(format!("monero setup: {what}: {e}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {

    pub installed: bool,

    pub wallet_exists: bool,

    pub running: bool,
    pub version: String,
}

pub fn install_dir(app_data: &Path) -> PathBuf {
    app_data.join("monero")
}

#[cfg(not(target_os = "android"))]
fn binary(app_data: &Path) -> PathBuf {
    install_dir(app_data).join(INNER).join(RPC_BIN)
}

fn installed_binary(app_data: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        let _ = app_data;
        crate::bundled::executable(RPC_BIN)
    }
    #[cfg(not(target_os = "android"))]
    {
        let exe = binary(app_data);
        exe.is_file().then_some(exe)
    }
}

fn wallet_dir(app_data: &Path) -> PathBuf {
    app_data.join("monero-wallet")
}

fn wallet_path(app_data: &Path) -> PathBuf {
    wallet_dir(app_data).join("account")
}

pub fn state(app_data: &Path, running: bool) -> SetupState {
    SetupState {
        installed: installed_binary(app_data).is_some(),
        wallet_exists: wallet_path(app_data).is_file(),
        running,
        version: VERSION.to_string(),
    }
}

#[cfg(target_os = "android")]
pub async fn install(app_data: &Path) -> Result<()> {
    if installed_binary(app_data).is_some() {
        return Ok(());
    }

    let why = if cfg!(any(target_arch = "aarch64", target_arch = "arm")) {
        "This copy of ShinyFlakes was built without Monero's wallet daemon inside it, \
         and Android does not let an app fetch a program afterwards. Build it with \
         scripts/android-binaries.mjs in place."
    } else {
        "Monero publishes its Android wallet daemon for ARM phones only, and this \
         device is not one."
    };
    Err(WalletError::Unsupported(why.into()))
}

#[cfg(not(target_os = "android"))]
pub async fn install(app_data: &Path) -> Result<()> {
    if binary(app_data).is_file() {
        return Ok(());
    }

    let dir = install_dir(app_data);
    std::fs::create_dir_all(&dir).map_err(|e| oops("creating the folder", e))?;

    let bytes = crate::http_client::builder()
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

    unpack(&dir, &bytes)?;

    let exe = binary(app_data);
    if !exe.is_file() {
        return Err(oops("unpacking", "the wallet daemon was not in the archive"));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| oops("permissions", e))?;
    }

    Ok(())
}

#[cfg(windows)]
fn unpack(dir: &Path, bytes: &[u8]) -> Result<()> {
    let archive = dir.join(ARCHIVE);
    std::fs::write(&archive, bytes).map_err(|e| oops("saving the archive", e))?;

    let file = std::fs::File::open(&archive).map_err(|e| oops("opening the archive", e))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| oops("reading the archive", e))?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| oops("reading an entry", e))?;

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
    Ok(())
}

#[cfg(target_os = "linux")]
fn unpack(dir: &Path, bytes: &[u8]) -> Result<()> {
    let bz = bzip2::read::BzDecoder::new(std::io::Cursor::new(bytes));
    let mut archive = tar::Archive::new(bz);

    for entry in archive.entries().map_err(|e| oops("reading the archive", e))? {
        let mut entry = entry.map_err(|e| oops("reading an entry", e))?;
        let relative = entry
            .path()
            .map_err(|e| oops("reading an entry", e))?
            .into_owned();

        if relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let target = dir.join(&relative);

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| oops("creating a folder", e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| oops("creating a folder", e))?;
        }
        entry.unpack(&target).map_err(|e| oops("writing a file", e))?;
    }
    Ok(())
}

pub fn spawn(app_data: &Path, daemon: &str) -> Result<Child> {
    let Some(exe) = installed_binary(app_data) else {
        return Err(oops("start", "the Monero wallet daemon is not installed"));
    };

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

        .arg("--log-level")
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

    #[cfg(target_os = "android")]
    {
        let logs = install_dir(app_data);
        std::fs::create_dir_all(&logs).map_err(|e| oops("creating the log folder", e))?;
        command
            .arg("--log-file")
            .arg(logs.join("monero-wallet-rpc.log"))
            .arg("--max-log-file-size")
            .arg("1048576")
            .arg("--max-log-files")
            .arg("2")
            .current_dir(&dir)
            .env("HOME", app_data);
        crate::bundled::keep_descriptors_private(&mut command);
    }

    command.spawn().map_err(|e| oops("start", e))
}

fn pid_file(app_data: &Path) -> PathBuf {
    install_dir(app_data).join("daemon.pid")
}

fn kill_stale(app_data: &Path) {
    let path = pid_file(app_data);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };

    if let Ok(pid) = text.trim().parse::<u32>() {

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

pub fn stop_any(app_data: &Path) {
    kill_stale(app_data);
}

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

pub async fn wait_until_ready(endpoint: &str) -> Result<()> {
    const ATTEMPTS: u32 = if cfg!(target_os = "android") { 150 } else { 40 };
    for attempt in 0..ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if super::xmr_rpc::ping(endpoint).await.is_ok() {
            return Ok(());
        }
        if attempt == ATTEMPTS - 1 {
            break;
        }
    }
    Err(oops("start", "the wallet daemon did not come up in time"))
}

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

    let height = daemon_height(daemon)
        .await
        .unwrap_or(0)
        .saturating_sub(RESTORE_SLACK_BLOCKS);

    super::xmr_rpc::generate_from_keys(
        endpoint, "account", address, spend_key, view_key, password, height,
    )
    .await?;

    Ok(())
}

async fn daemon_height(daemon: &str) -> Result<u64> {
    #[derive(serde::Deserialize)]
    struct Info {
        height: u64,
    }

    let info: Info = crate::http_client::builder()
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

pub fn wallet_password() -> Result<String> {
    crate::keychain::monero_password()
}

pub fn write_readme(app_data: &Path) {
    let store = crate::keychain::STORE_NAME;
    let note = format!(
        "This folder holds a Monero wallet created by ShinyFlakes {VERSION}.\n\
         \n\
         It was restored from the keys this wallet derives from your seed\n\
         phrase. Its password is stored in the {store}.\n\
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

    #[cfg(not(target_os = "android"))]
    #[test]
    fn the_pinned_hash_is_a_sha256() {
        assert_eq!(SHA256.len(), 64);
        assert!(SHA256.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[cfg(not(target_os = "android"))]
    #[test]
    fn paths_stay_inside_the_app_folder() {
        let root = Path::new("/example/appdata");
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
