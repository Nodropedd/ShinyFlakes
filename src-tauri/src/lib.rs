//! App entry and setup.

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
mod update;

pub use error::WalletError;

use tauri::Manager;

pub fn now_unix() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    avoid_nvidia_wayland_crash();

    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());

    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {

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

            #[cfg(target_os = "android")]
            keychain::set_app_dir(dir.clone());

            app.manage(session::AppState::new(
                dir.join("wallet.vault"),
                dir.clone(),
            ));

            #[cfg(debug_assertions)]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_skip_taskbar(false);
                let _ = window.set_content_protected(false);
                let _ = window.show();
                let _ = window.set_focus();
            }

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
            ipc::check_for_update,
            ipc::open_download_page,
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

            if let tauri::RunEvent::Exit = event {
                let state = app.state::<session::AppState>();
                state.stop_monero();
                state.stop_tor();
            }
        });
}

#[cfg(target_os = "linux")]
fn avoid_nvidia_wayland_crash() {
    const VAR: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";
    let nvidia = std::path::Path::new("/sys/module/nvidia").exists();
    if nvidia && std::env::var_os(VAR).is_none() {

        std::env::set_var(VAR, "1");
    }
}
