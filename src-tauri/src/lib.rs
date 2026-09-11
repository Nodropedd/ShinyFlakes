mod appconfig;
mod chains;
mod crypto;
mod donation;
mod email;
mod error;
mod fee;
mod inactivity;
mod ipc;
mod keychain;
mod session;
mod store;
mod tor;
mod twofa;

pub use error::WalletError;

use tauri::Manager;

pub fn run() {
    tauri::Builder::default()
        // One wallet process at a time. A second launch focuses the window
        // that already exists rather than opening a second copy against the
        // same vault file.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
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

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::vault_status,
            ipc::generate_mnemonic,
            ipc::create_vault,
            ipc::unlock,
            ipc::lock,
            ipc::logout,
            ipc::forget_wallet,
            ipc::list_buckets,
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
            ipc::email_config,
            ipc::set_email_config,
            ipc::send_test_email,
            ipc::set_two_factor,
            ipc::two_factor_state,
            ipc::request_2fa,
            ipc::verify_2fa,
            ipc::verify_2fa_seed,
            ipc::reveal_seed,
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
