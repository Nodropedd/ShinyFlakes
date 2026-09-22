mod appconfig;
#[cfg(target_os = "android")]
mod bundled;
mod chains;
mod crypto;
mod donation;
mod error;
mod fee;
mod http_client;
mod inactivity;
mod ipc;
mod keychain;
mod session;
mod store;
mod swapcfg;
mod tor;
mod totp;
mod twofa;

pub use error::WalletError;

use tauri::Manager;

/// The current Unix time in seconds, shared by the modules that gate on it.
pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// On Android there is no `main`: the Java side loads this library and calls
/// in, so the entry point has to be exported under the name it looks for.
/// The macro does nothing on desktop, where `main.rs` still calls this.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    avoid_nvidia_wayland_crash();

    let builder = tauri::Builder::default();

    // One wallet process at a time. A second launch focuses the window that
    // already exists rather than opening a second copy against the same vault
    // file. Desktop only: Android has one instance by construction, and the
    // plugin does not build for it.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        // Focus alone does nothing for a window that is minimised or hidden,
        // and GNOME refuses to raise a window for an app that asks for itself
        // — it flags it as wanting attention instead. So bring it back into
        // view first; then at worst the user clicks the notice.
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));

    builder
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;

            // Android has no credential store to address by name, so its
            // key backend needs to be told where this app may write. See
            // keychain::android_store for what that costs.
            #[cfg(target_os = "android")]
            keychain::set_app_dir(dir.clone());

            app.manage(session::AppState::new(
                dir.join("wallet.vault"),
                dir.clone(),
            ));

            // Release keeps what tauri.conf.json asks for: no taskbar button,
            // and content protection so the window is blank to screen capture.
            // Both make the app impossible to find or screenshot while
            // developing, so debug builds turn them off and show the window.
            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
                let _ = window.set_content_protected(false);
                let _ = window.show();
                let _ = window.set_focus();
            }

            // Linux keeps its taskbar entry, release or not. On X11 the hint
            // is honoured by hiding the window from GNOME's dash, Alt+Tab and
            // overview alike, so the first time anything covered it there was
            // no way back — and relaunching only handed over to the lost
            // window. On Wayland GTK ignores the hint entirely. Windows keeps
            // it: there it removes the taskbar button and Alt+Tab still works.
            #[cfg(target_os = "linux")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::vault_status,
            ipc::generate_mnemonic,
            ipc::create_vault,
            ipc::unlock,
            ipc::lock,
            ipc::auto_unlock,
            ipc::stay_signed_in_state,
            ipc::set_stay_signed_in,
            ipc::logout,
            ipc::forget_wallet,
            ipc::list_addresses,
            ipc::fetch_balances,
            ipc::fetch_prices,
            ipc::fetch_activity,
            ipc::send_limits,
            ipc::send_preview,
            ipc::send_execute,
            ipc::utxo_state,
            ipc::fragment_preview,
            ipc::fragment_execute,
            ipc::consolidate_preview,
            ipc::consolidate_execute,
            ipc::list_spendable,
            ipc::next_receive_address,
            ipc::reveal_monero_keys,
            ipc::xmr_status,
            ipc::xmr_balance,
            ipc::xmr_preview,
            ipc::xmr_send,
            ipc::monero_setup_state,
            ipc::monero_setup_run,
            ipc::monero_stop,
            ipc::donation_addresses,
            ipc::tor_state,
            ipc::tor_start,
            ipc::tor_stop,
            ipc::inactivity_check,
            ipc::inactivity_set_months,
            ipc::inactivity_set_action,
            ipc::inactivity_sweep,
            ipc::set_vault_passphrase,
            ipc::two_factor_state,
            ipc::begin_totp_setup,
            ipc::confirm_totp,
            ipc::settings_unreadable,
            ipc::reset_settings,
            ipc::step_up_settings,
            ipc::set_step_up_settings,
            ipc::step_up_challenge,
            ipc::dormant_step_up_pending,
            ipc::clear_dormant_step_up,
            ipc::disable_two_factor,
            ipc::verify_2fa,
            ipc::verify_2fa_seed,
            ipc::reveal_seed,
            ipc::swap_quote,
            ipc::swap_create,
            ipc::swap_fund,
            ipc::swap_status,
        ])
        .build(tauri::generate_context!())
        .expect("ShinyFlakes failed to start")
        .run(|app, event| {
            // The Monero daemon is a child process. Without this it would
            // outlive the window that started it, holding an open wallet.
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<session::AppState>();
                state.stop_monero();
                state.stop_tor();
            }
        });
}


/// WebKitGTK's DMA-BUF renderer and NVIDIA's driver disagree on Wayland: the
/// first frame ends in "Error 71 (Protocol error) dispatching to Wayland
/// display" and GTK quits, so the window flashes up and is gone. Rendering
/// without DMA-BUF avoids it at a small cost in compositing speed, which a
/// wallet never notices. Only with NVIDIA's driver loaded, where the fault
/// is, and never over a choice the user made in the environment already.
#[cfg(target_os = "linux")]
fn avoid_nvidia_wayland_crash() {
    const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    let nvidia = std::path::Path::new("/sys/module/nvidia").exists();
    if nvidia && std::env::var_os(VAR).is_none() {
        // Before GTK, WebKit or any other thread exists, which is the only
        // time changing the environment is safe.
        std::env::set_var(VAR, "1");
    }
}
