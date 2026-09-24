//! Commands the UI can call.

use serde::Serialize;
use tauri::State;
use zeroize::Zeroizing;

use crate::chains::rpc::AssetBalance;
use crate::chains::{self, AssetAddress};
use std::collections::HashMap;
use crate::crypto::{ct_eq, seed};
use crate::error::{Result, WalletError};
use crate::keychain;
use crate::session::{AppState, Unlocked};
use crate::store::{self, VaultPayload};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub initialized: bool,
    pub unlocked: bool,

    pub needs_passphrase: bool,

    pub key_missing: bool,
}

#[tauri::command]
pub fn vault_status(state: State<AppState>) -> VaultStatus {
    let initialized = store::exists(&state.vault_path);

    let key_missing =
        initialized && matches!(keychain::load(), Err(WalletError::NoVault));
    VaultStatus {
        initialized,
        unlocked: state.is_unlocked(),
        needs_passphrase: initialized
            && store::lock_info(&state.vault_path)
                .map(|i| i.needs_passphrase)
                .unwrap_or(false),
        key_missing,
    }
}

#[tauri::command]
pub fn generate_mnemonic() -> Result<String> {
    Ok(seed::generate()?.to_string())
}

#[tauri::command]
pub fn create_vault(mnemonic: String, fresh: Option<bool>, state: State<AppState>) -> Result<()> {
    let phrase = Zeroizing::new(mnemonic);

    if store::exists(&state.vault_path) {
        return Err(WalletError::VaultExists);
    }

    let parsed = seed::parse(&phrase)?;
    let key = keychain::load_or_create()?;
    if crate::appconfig::is_unreadable(&state.data_dir) {
        crate::appconfig::discard(&state.data_dir);
    }

    let payload = VaultPayload {
        mnemonic: parsed.to_string(),
    };

    store::write(&state.vault_path, &key, None, &payload)?;

    let mut guard = state
        .unlocked
        .lock()
        .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
    *guard = Some(Unlocked {
        seed: seed::to_seed(&parsed),
        mnemonic: Zeroizing::new(parsed.to_string()),
    });

    drop(guard);
    if fresh == Some(true) {
        newest_prefs(&state.data_dir, seed::to_seed(&parsed).as_ref());
    } else {
        apply_wallet_prefs(&state.data_dir, seed::to_seed(&parsed).as_ref());
    }
    settle_dormancy_and_record_use(&state);
    resume_staying_signed_in(&state);

    Ok(())
}

fn settle_dormancy_and_record_use(state: &AppState) {

    let dormant = match crate::appconfig::load(&state.data_dir) {
        Ok(cfg) => {
            crate::twofa::dormant(
                crate::inactivity::last_seen(&state.data_dir),
                cfg.step_up.dormant_days,
                crate::now_unix(),
            ) && crate::twofa::required(&cfg, crate::twofa::Guarded::Dormant).any()
        }
        Err(_) => false,
    };
    if let Ok(mut tf) = state.two_factor.lock() {
        tf.dormant_pending = dormant;
    }

    let _ = crate::inactivity::record_seen(&state.data_dir);
}

#[tauri::command]
pub fn unlock(
    mnemonic: String,
    passphrase: Option<String>,
    state: State<AppState>,
) -> Result<()> {
    let phrase = Zeroizing::new(mnemonic);

    if !store::exists(&state.vault_path) {
        return Err(WalletError::NoVault);
    }

    let parsed = seed::parse(&phrase)?;

    let key = keychain::load().map_err(|e| match e {
        WalletError::NoVault => WalletError::KeyMissing,
        other => other,
    })?;
    let pass = passphrase.as_deref().filter(|p| !p.is_empty());
    let payload = store::read(&state.vault_path, &key, pass)?;

    if !ct_eq(parsed.to_string().as_bytes(), payload.mnemonic.as_bytes()) {
        return Err(WalletError::WrongSeed);
    }

    let mut guard = state
        .unlocked
        .lock()
        .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
    *guard = Some(Unlocked {
        seed: seed::to_seed(&parsed),
        mnemonic: Zeroizing::new(parsed.to_string()),
    });
    drop(guard);
    apply_wallet_prefs(&state.data_dir, seed::to_seed(&parsed).as_ref());

    let _ = crate::inactivity::record_seen(&state.data_dir);
    resume_staying_signed_in(&state);

    Ok(())
}

#[tauri::command]
pub fn lock(state: State<AppState>) {
    pause_staying_signed_in(&state);
    state.wipe();
}

#[tauri::command]
pub fn logout(state: State<AppState>) {
    pause_staying_signed_in(&state);
    state.wipe();
}

#[tauri::command]
pub async fn check_for_update() -> Result<crate::update::UpdateInfo> {
    crate::update::check().await
}

#[tauri::command]
pub fn open_download_page(app: tauri::AppHandle) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(crate::update::DOWNLOAD_PAGE, None::<&str>)
        .map_err(|e| WalletError::Network(format!("could not open the download page: {e}")))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaySignedIn {

    pub enabled: bool,

    pub available: bool,
}

fn has_passphrase(state: &AppState) -> bool {
    store::lock_info(&state.vault_path)
        .map(|i| i.needs_passphrase)
        .unwrap_or(false)
}

fn staying_signed_in(state: &AppState) -> StaySignedIn {
    let available = !has_passphrase(state);
    let enabled = crate::appconfig::load(&state.data_dir)
        .map(|c| c.stay_signed_in)
        .unwrap_or(false);
    StaySignedIn {
        enabled: enabled && available,
        available,
    }
}

fn pause_staying_signed_in(state: &AppState) {
    if let Ok(mut cfg) = crate::appconfig::load(&state.data_dir) {
        if cfg.stay_signed_in && !cfg.sign_in_paused {
            cfg.sign_in_paused = true;
            let _ = crate::appconfig::save(&state.data_dir, &cfg);
        }
    }
}

fn resume_staying_signed_in(state: &AppState) {
    if let Ok(mut cfg) = crate::appconfig::load(&state.data_dir) {
        if cfg.sign_in_paused {
            cfg.sign_in_paused = false;
            let _ = crate::appconfig::save(&state.data_dir, &cfg);
        }
    }
}

#[tauri::command]
pub fn stay_signed_in_state(state: State<AppState>) -> StaySignedIn {
    staying_signed_in(&state)
}

#[tauri::command]
pub fn set_stay_signed_in(on: bool, state: State<AppState>) -> Result<StaySignedIn> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    if on {
        if has_passphrase(&state) {
            return Err(WalletError::Unsupported(
                "Staying signed in means the wallet opens without anything typed, and a \
                 vault passphrase has to be typed every time. Remove the passphrase first."
                    .into(),
            ));
        }
        require_2fa(&state)?;
    }

    let mut cfg = crate::appconfig::load(&state.data_dir)?;
    cfg.stay_signed_in = on;
    cfg.sign_in_paused = false;
    crate::appconfig::save(&state.data_dir, &cfg)?;
    Ok(staying_signed_in(&state))
}

#[tauri::command]
pub fn auto_unlock(state: State<AppState>) -> Result<bool> {
    if state.is_unlocked() {
        return Ok(true);
    }
    if !store::exists(&state.vault_path) {
        return Ok(false);
    }

    let Ok(key) = keychain::load() else {
        return Ok(false);
    };
    let Ok(cfg) = crate::appconfig::load(&state.data_dir) else {
        return Ok(false);
    };
    if !cfg.stay_signed_in || cfg.sign_in_paused {
        return Ok(false);
    }

    let Ok(payload) = store::read(&state.vault_path, &key, None) else {
        return Ok(false);
    };
    let Ok(parsed) = seed::parse(&payload.mnemonic) else {
        return Ok(false);
    };

    {
        let mut guard = state
            .unlocked
            .lock()
            .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
        *guard = Some(Unlocked {
            seed: seed::to_seed(&parsed),
            mnemonic: Zeroizing::new(parsed.to_string()),
        });
    }
    apply_wallet_prefs(&state.data_dir, seed::to_seed(&parsed).as_ref());

    settle_dormancy_and_record_use(&state);
    Ok(true)
}

#[tauri::command]
pub fn list_addresses(state: State<AppState>) -> Result<Vec<AssetAddress>> {
    let guard = state
        .unlocked
        .lock()
        .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;

    match guard.as_ref() {
        Some(session) => chains::addresses(session.seed.as_ref()),
        None => Err(WalletError::Locked),
    }
}

fn wallet_id(seed: &[u8]) -> Option<String> {
    chains::sol_address(seed, false).ok()
}

fn load_prefs(data_dir: &std::path::Path, seed: &[u8]) -> crate::appconfig::WalletPrefs {
    let id = wallet_id(seed).unwrap_or_default();
    let cfg = crate::appconfig::load(data_dir).unwrap_or_default();
    match cfg.wallet_prefs {
        Some(p) if p.id == id => p,
        _ => crate::appconfig::WalletPrefs {
            sol_exodus: (cfg.sol_exodus_for.as_deref() == Some(id.as_str())).then_some(true),
            id,
            ..Default::default()
        },
    }
}

fn save_prefs(data_dir: &std::path::Path, prefs: &crate::appconfig::WalletPrefs) {
    if let Ok(mut cfg) = crate::appconfig::load(data_dir) {
        cfg.wallet_prefs = Some(prefs.clone());
        let _ = crate::appconfig::save(data_dir, &cfg);
    }
}

fn use_prefs(prefs: &crate::appconfig::WalletPrefs) {
    use chains::btc_tx::{set_preferred, Chain, Kind};
    chains::set_sol_exodus(prefs.sol_exodus.unwrap_or(false));
    set_preferred(Chain::Bitcoin, Kind::from_code(prefs.btc.unwrap_or(0)));
    set_preferred(Chain::Litecoin, Kind::from_code(prefs.ltc.unwrap_or(0)));
}

pub fn apply_wallet_prefs(data_dir: &std::path::Path, seed: &[u8]) {
    use_prefs(&load_prefs(data_dir, seed));
}

fn newest_prefs(data_dir: &std::path::Path, seed: &[u8]) {
    let prefs = crate::appconfig::WalletPrefs {
        id: wallet_id(seed).unwrap_or_default(),
        sol_exodus: Some(false),
        btc: Some(0),
        ltc: Some(0),
    };
    save_prefs(data_dir, &prefs);
    use_prefs(&prefs);
}

fn richest_kind(coins: &[chains::btc_tx::Utxo]) -> Option<u8> {
    use chains::btc_tx::Kind;
    let total = |k: Kind| coins.iter().filter(|c| c.kind == k).map(|c| c.value).sum::<u64>();
    let best = Kind::ALL.into_iter().max_by_key(|k| (total(*k), std::cmp::Reverse(Kind::ALL.iter().position(|x| x == k))))?;
    (total(best) > 0).then(|| best.code())
}

async fn settle_sol_scheme(data_dir: &std::path::Path, seed: &[u8; 64]) {
    let mut prefs = load_prefs(data_dir, seed);
    if prefs.sol_exodus.is_some() {
        return;
    }
    let (Ok(phantom), Ok(exodus)) = (chains::sol_address(seed, false), chains::sol_address(seed, true))
    else {
        return;
    };
    let (p, e) = tokio::join!(
        chains::rpc::sol_balance_of(&phantom),
        chains::rpc::sol_balance_of(&exodus)
    );
    if let (Ok(p), Ok(e)) = (p, e) {
        if e > p || p > 0 {
            prefs.sol_exodus = Some(e > p);
            save_prefs(data_dir, &prefs);
            use_prefs(&prefs);
        }
    }
}

fn settle_utxo_kinds(
    data_dir: &std::path::Path,
    seed: &[u8; 64],
    btc: Option<&[chains::btc_tx::Utxo]>,
    ltc: Option<&[chains::btc_tx::Utxo]>,
) {
    let mut prefs = load_prefs(data_dir, seed);
    let before = prefs.clone();
    if let (None, Some(coins)) = (prefs.btc, btc) {
        prefs.btc = richest_kind(coins);
    }
    if let (None, Some(coins)) = (prefs.ltc, ltc) {
        prefs.ltc = richest_kind(coins);
    }
    if prefs != before {
        save_prefs(data_dir, &prefs);
        use_prefs(&prefs);
    }
}

#[tauri::command]
pub async fn fetch_balances(state: State<'_, AppState>) -> Result<Vec<AssetBalance>> {
    let seed = seed_copy(&state)?;
    settle_sol_scheme(&state.data_dir, &seed).await;
    let addresses = chains::addresses(seed.as_ref())?;
    let (mut out, btc, ltc) = tokio::join!(
        chains::rpc::balances(&addresses),
        scan_coins(&seed, chains::btc_tx::Chain::Bitcoin, false),
        scan_coins(&seed, chains::btc_tx::Chain::Litecoin, false),
    );
    settle_utxo_kinds(
        &state.data_dir,
        &seed,
        btc.as_ref().ok().map(|(_, c)| c.as_slice()),
        ltc.as_ref().ok().map(|(_, c)| c.as_slice()),
    );
    for (asset, scanned) in [("BTC", btc), ("LTC", ltc)] {
        if let (Ok((_, coins)), Some(entry)) = (scanned, out.iter_mut().find(|b| b.asset == asset)) {
            let total: u64 = coins.iter().map(|c| c.value).sum();
            entry.minor = Some(total.to_string());
            entry.error = None;
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn fetch_prices(currency: String) -> Result<HashMap<String, chains::rpc::Quote>> {
    chains::rpc::prices(&currency).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendQuote {
    pub asset: String,
    pub to: String,
    pub amount_minor: String,
    pub fee_minor: String,

    pub creator_fee_minor: String,
    pub total_minor: String,

    pub simulated: bool,
}

fn creator_fee_minor(asset: &str, amount_minor: u128, usd: Option<f64>, to: &str) -> u128 {
    match donation_for(asset) {
        Some(addr) => crate::fee::resolve(amount_minor, usd, to, &addr),
        None => 0,
    }
}

fn seed_copy(state: &State<AppState>) -> Result<[u8; 64]> {
    let guard = state
        .unlocked
        .lock()
        .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
    match guard.as_ref() {
        Some(session) => Ok(*session.seed),
        None => Err(WalletError::Locked),
    }
}

fn btc_chain(asset: &str) -> Option<chains::btc_tx::Chain> {
    match asset {
        "BTC" => Some(chains::btc_tx::Chain::Bitcoin),
        "LTC" => Some(chains::btc_tx::Chain::Litecoin),
        _ => None,
    }
}

async fn scan_stream(
    seed: &[u8; 64],
    chain: chains::btc_tx::Chain,
    kind: chains::btc_tx::Kind,
    change: bool,
    gap: u32,
    confirmed_only: bool,
) -> Result<(Vec<chains::btc_tx::Keys>, Vec<chains::btc_tx::Utxo>)> {
    use chains::rpc::{SCAN_BATCH, SCAN_CEILING, SCAN_PARALLEL};
    let apis = chains::rpc::btc_apis(chain);
    let mut keyring: Vec<chains::btc_tx::Keys> = Vec::new();
    let mut utxos: Vec<chains::btc_tx::Utxo> = Vec::new();
    let mut next = 0u32;
    let mut empty_run = 0u32;
    while next < SCAN_CEILING && empty_run < gap {
        let mut batch = Vec::new();
        for _ in 0..SCAN_BATCH.min(gap).min(SCAN_PARALLEL) {
            if next >= SCAN_CEILING {
                break;
            }
            let keys = chains::btc_tx::keys_on(seed, chain, kind, change, next)?;
            batch.push((keyring.len() as u32, chains::btc_tx::address_of(&keys, chain)?));
            keyring.push(keys);
            next += 1;
        }
        for (_, mut found) in chains::rpc::esplora_utxos_batch(apis, &batch, confirmed_only).await? {
            if found.is_empty() {
                empty_run += 1;
            } else {
                empty_run = 0;
                for utxo in &mut found {
                    utxo.kind = kind;
                }
                utxos.append(&mut found);
            }
        }
    }
    Ok((keyring, utxos))
}

async fn scan_coins(
    seed: &[u8; 64],
    chain: chains::btc_tx::Chain,
    confirmed_only: bool,
) -> Result<(Vec<chains::btc_tx::Keys>, Vec<chains::btc_tx::Utxo>)> {
    use chains::btc_tx::Kind;
    use chains::rpc::{EXTRA_GAP, SCAN_GAP};
    let mut streams = vec![(Kind::Segwit, false, SCAN_GAP), (Kind::Segwit, true, EXTRA_GAP)];
    for kind in [Kind::Nested, Kind::Legacy] {
        streams.push((kind, false, EXTRA_GAP));
        streams.push((kind, true, EXTRA_GAP));
    }

    let mut keyring: Vec<chains::btc_tx::Keys> = Vec::new();
    let mut utxos: Vec<chains::btc_tx::Utxo> = Vec::new();
    for (kind, change, gap) in streams {
        let (keys, mut found) = scan_stream(seed, chain, kind, change, gap, confirmed_only).await?;
        let offset = keyring.len() as u32;
        for utxo in &mut found {
            utxo.key_index += offset;
        }
        keyring.extend(keys);
        utxos.append(&mut found);
    }
    utxos.sort_by(|a, b| b.value.cmp(&a.value));
    Ok((keyring, utxos))
}

fn home_keys(seed: &[u8; 64], chain: chains::btc_tx::Chain) -> Result<chains::btc_tx::Keys> {
    chains::btc_tx::keys_on(seed, chain, chains::btc_tx::preferred(chain), false, 0)
}

fn home_script(seed: &[u8; 64], chain: chains::btc_tx::Chain) -> Result<Vec<u8>> {
    chains::btc_tx::script_of(&home_keys(seed, chain)?, chain)
}

async fn btc_context(
    seed: &[u8; 64],
    chain: chains::btc_tx::Chain,
) -> Result<(Vec<chains::btc_tx::Keys>, Vec<chains::btc_tx::Utxo>, f64)> {
    let (keyring, utxos) = scan_coins(seed, chain, true).await?;
    let rate = chains::rpc::esplora_fee_rate(chains::rpc::btc_apis(chain)).await?;
    Ok((keyring, utxos, rate))
}

fn parse_wei(amount: &str) -> Result<u128> {
    amount
        .trim()
        .parse::<u128>()
        .map_err(|_| WalletError::Derivation(format!("amount is not a whole number: {amount}")))
}

async fn eth_context(seed: &[u8; 64]) -> Result<(chains::eth::Keys, String, u128, u128, u128)> {
    let keys = chains::eth::keys(seed)?;
    let address = chains::eth::to_checksum(&keys.address);
    let balance = chains::rpc::eth_balance(&address).await?;
    let (max_fee, tip) = chains::rpc::eth_fees().await?;
    Ok((keys, address, balance, max_fee, tip))
}

fn parse_minor(amount: &str) -> Result<u64> {
    amount
        .trim()
        .parse::<u64>()
        .map_err(|_| WalletError::Derivation(format!("amount is not a whole number: {amount}")))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendLimits {
    pub asset: String,
    pub balance_minor: String,
    pub fee_minor: String,

    pub max_minor: String,

    pub rent_minimum_minor: String,
}

#[tauri::command]
pub async fn send_limits(
    asset: String,
    network: Option<String>,
    state: State<'_, AppState>,
) -> Result<SendLimits> {
    let seed = seed_copy(&state)?;

    if let Some(chain) = btc_chain(&asset) {
        let (_keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let balance: u64 = utxos.iter().map(|u| u.value).sum();
        let max = chains::btc_tx::max_sendable(&utxos, rate);
        return Ok(SendLimits {
            asset,
            balance_minor: balance.to_string(),
            fee_minor: balance.saturating_sub(max).to_string(),
            max_minor: max.to_string(),

            rent_minimum_minor: chains::btc_tx::DUST.to_string(),
        });
    }

    if asset == "ETH" {
        let (_keys, _address, balance, max_fee, _tip) = eth_context(&seed).await?;
        let cost = max_fee * chains::eth::TRANSFER_GAS as u128;
        return Ok(SendLimits {
            asset,
            balance_minor: balance.to_string(),
            fee_minor: cost.to_string(),
            max_minor: balance.saturating_sub(cost).to_string(),

            rent_minimum_minor: "0".into(),
        });
    }

    if asset == "TRON" {
        let address = chains::tron_tx::address(&seed)?;
        let balance = chains::rpc::tron_trx_balance(&address).await?;
        let reserve = chains::tron_tx::FEE_RESERVE_SUN;
        return Ok(SendLimits {
            asset,
            balance_minor: balance.to_string(),
            fee_minor: reserve.to_string(),
            max_minor: balance.saturating_sub(reserve).to_string(),
            rent_minimum_minor: "0".into(),
        });
    }

    if chains::tokens::is_token(&asset) {
        let net = token_network(&asset, &network)?;
        let balance = token_balance(&seed, &asset, &net).await?;

        return Ok(SendLimits {
            asset,
            balance_minor: balance.to_string(),
            fee_minor: "0".into(),
            max_minor: balance.to_string(),
            rent_minimum_minor: "0".into(),
        });
    }

    if asset != "SOL" {
        return Err(WalletError::Unsupported(format!(
            "Sending {asset} is not implemented yet."
        )));
    }

    let address = bs58::encode(
        chains::sol_tx::signing_key(&seed)?
            .verifying_key()
            .to_bytes(),
    )
    .into_string();

    let balance = chains::rpc::sol_balance_of(&address).await?.max(0) as u64;
    let rent = chains::rpc::sol_rent_exempt_minimum().await?;

    let fee = chains::sol_tx::LAMPORTS_PER_SIGNATURE;

    Ok(SendLimits {
        asset,
        balance_minor: balance.to_string(),
        fee_minor: fee.to_string(),
        max_minor: balance.saturating_sub(fee).to_string(),
        rent_minimum_minor: rent.to_string(),
    })
}

async fn check_affordable(seed: &[u8; 64], amount: u64) -> Result<u64> {
    let address = bs58::encode(
        chains::sol_tx::signing_key(seed)?.verifying_key().to_bytes(),
    )
    .into_string();

    let balance = chains::rpc::sol_balance_of(&address).await?.max(0) as u64;
    let fee = chains::sol_tx::LAMPORTS_PER_SIGNATURE;

    let needed = amount as u128 + fee as u128;
    if needed > balance as u128 {
        return Err(WalletError::Funds(format!(
            "This needs {} lamports including the {fee} lamport fee, but the              account holds {balance}. The most you can send is {}.",
            needed,
            balance.saturating_sub(fee)
        )));
    }

    let remainder = balance - amount - fee;
    if remainder > 0 {
        let rent = chains::rpc::sol_rent_exempt_minimum().await?;
        if remainder < rent {
            return Err(WalletError::Funds(format!(
                "That would leave {remainder} lamports behind, and Solana will                  not let an account hold less than {rent} without being closed.                  Either send at most {} and keep the account open, or send                  exactly {} to empty it.",
                balance.saturating_sub(fee).saturating_sub(rent),
                balance.saturating_sub(fee)
            )));
        }
    }

    Ok(fee)
}

async fn tron_sign_and_send(
    seed: &[u8; 64],
    tx: serde_json::Value,
    expected: &[String],
) -> Result<String> {
    let raw_hex = tx
        .get("raw_data_hex")
        .and_then(|h| h.as_str())
        .ok_or_else(|| WalletError::Network("Tron returned no raw data".into()))?
        .to_lowercase();
    let txid_hex = tx
        .get("txID")
        .and_then(|h| h.as_str())
        .ok_or_else(|| WalletError::Network("Tron returned no transaction id".into()))?
        .to_lowercase();

    for want in expected {
        if !raw_hex.contains(want.as_str()) {
            return Err(WalletError::Network(
                "Tron returned a transaction that does not match this transfer.".into(),
            ));
        }
    }
    let raw = chains::tron_tx::unhex(&raw_hex)?;
    let id = chains::tron_tx::txid(&raw);
    if chains::tron_tx::hex(&id) != txid_hex {
        return Err(WalletError::Network(
            "Tron's transaction id does not match its raw data.".into(),
        ));
    }

    let key = chains::tron_tx::signing_key(seed)?;
    let sig = chains::tron_tx::sign_txid(&key, &id)?;
    let mut signed = tx;
    signed["signature"] = serde_json::json!([chains::tron_tx::hex(&sig)]);

    let broadcast = chains::rpc::tron_broadcast(&signed).await?;
    Ok(if broadcast.is_empty() { txid_hex } else { broadcast })
}

async fn tron_send_trx(seed: &[u8; 64], to: &str, amount: u64) -> Result<String> {
    let owner_bytes = chains::tron_tx::account_bytes(seed)?;
    let owner = chains::tron_tx::address(seed)?;
    let to = to.trim();
    let to_bytes = chains::tron_tx::parse_address(to)?;

    let tx = chains::rpc::tron_create_transfer(&owner, to, amount).await?;
    let expected =
        chains::tron_tx::hex(&chains::tron_tx::transfer_value(&owner_bytes, &to_bytes, amount));
    tron_sign_and_send(seed, tx, &[expected]).await
}

async fn tron_send_trc20(
    seed: &[u8; 64],
    contract: &str,
    to: &str,
    amount: u128,
) -> Result<String> {
    const FEE_LIMIT: u64 = 100_000_000;
    let owner = chains::tron_tx::address(seed)?;
    let to = to.trim();
    let to_bytes = chains::tron_tx::parse_address(to)?;
    let to20 = chains::tron_tx::body20(&to_bytes);

    let param_hex = chains::tron_tx::hex(&chains::tron_tx::trc20_parameter(&to20, amount));
    let tx = chains::rpc::tron_trigger(
        &owner,
        contract,
        "transfer(address,uint256)",
        &param_hex,
        FEE_LIMIT,
    )
    .await?;

    let data_hex = format!("a9059cbb{param_hex}");
    let contract_hex = chains::tron_tx::hex(&chains::tron_tx::parse_address(contract)?);
    tron_sign_and_send(seed, tx, &[data_hex, contract_hex]).await
}

fn sol_own_address(seed: &[u8; 64]) -> Result<String> {
    chains::sol_address(seed, chains::sol_exodus())
}

fn hexstr(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn spl_balance(seed: &[u8; 64], mint: &str) -> Result<u128> {
    chains::rpc::sol_spl_balance(&sol_own_address(seed)?, mint).await
}

async fn spl_send(seed: &[u8; 64], mint: &str, to: &str, amount: u64) -> Result<String> {
    let owner = sol_own_address(seed)?;
    let owner_key = chains::sol_tx::signing_key(seed)?.verifying_key().to_bytes();
    let mint_key = chains::sol_tx::parse_address(mint)?;
    let derived =
        bs58::encode(chains::sol_tx::associated_token_account(&owner_key, &mint_key)).into_string();

    let accounts = chains::rpc::sol_token_accounts(&owner, mint).await?;
    let have: u128 = accounts
        .iter()
        .find(|(pk, _)| *pk == derived)
        .map(|(_, amt)| *amt)
        .ok_or_else(|| {
            WalletError::Funds(
                "This wallet has no verified token account for that coin on Solana, so there \
                 is nothing to send. If you just received it, wait for the balance to appear."
                    .into(),
            )
        })?;
    if amount as u128 > have {
        return Err(WalletError::Funds(format!(
            "This account holds {have} of that token on Solana, less than {amount}."
        )));
    }

    let blockhash = chains::rpc::sol_latest_blockhash().await?;
    let tx = chains::sol_tx::signed_spl_transfer(
        seed,
        to,
        mint,
        amount,
        chains::tokens::TOKEN_DECIMALS as u8,
        &blockhash,
    )?;

    chains::rpc::sol_simulate(&tx).await?;
    chains::rpc::sol_broadcast(&tx).await
}

async fn erc20_balance(seed: &[u8; 64], contract: &str) -> Result<u128> {
    let keys = chains::eth::keys(seed)?;
    let data = format!("0x{}", hexstr(&chains::eth::erc20_balance_data(&keys.address)));
    chains::rpc::eth_view(contract, &data).await
}

async fn erc20_send(seed: &[u8; 64], contract: &str, to: &str, amount: u128) -> Result<String> {
    let (keys, address, _balance, max_fee, tip) = eth_context(seed).await?;
    let dest = chains::eth::parse_address(to)?;
    let contract_addr = chains::eth::parse_address(contract)?;

    let held = erc20_balance(seed, contract).await?;
    if amount > held {
        return Err(WalletError::Funds(format!(
            "This account holds {held} of that token, less than {amount}."
        )));
    }

    let gas_limit = 100_000u64;
    let nonce = chains::rpc::eth_nonce(&address).await?;
    let tx = chains::eth::Transfer {
        nonce,
        max_priority_fee: tip,
        max_fee,
        gas_limit,
        to: contract_addr,
        value: 0,
        data: chains::eth::erc20_transfer_data(&dest, amount),
    };
    let signed = chains::eth::sign(&keys, &tx)?;
    chains::rpc::eth_broadcast(&signed).await
}

fn token_network(asset: &str, network: &Option<String>) -> Result<String> {
    let chosen = network
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_uppercase);
    let net = match chosen {
        Some(n) => n,
        None => match asset {
            "USDC" => "SOL".to_string(),
            _ => "TRON".to_string(),
        },
    };
    if chains::tokens::contract(asset, &net).is_none() {
        return Err(WalletError::Unsupported(format!(
            "{asset} cannot be sent on {net}."
        )));
    }
    Ok(net)
}

async fn token_balance(seed: &[u8; 64], asset: &str, network: &str) -> Result<u128> {
    let contract = chains::tokens::contract(asset, network)
        .ok_or_else(|| WalletError::Unsupported(format!("{asset} is not on {network}")))?;
    match network {
        "SOL" => spl_balance(seed, contract).await,
        "ETH" => erc20_balance(seed, contract).await,
        "TRON" => chains::rpc::tron_trc20_balance(&chains::tron_tx::address(seed)?, contract).await,
        other => Err(WalletError::Unsupported(format!("unknown network {other}"))),
    }
}

async fn token_send(
    seed: &[u8; 64],
    asset: &str,
    network: &str,
    to: &str,
    amount: u128,
) -> Result<String> {
    let contract = chains::tokens::contract(asset, network)
        .ok_or_else(|| WalletError::Unsupported(format!("{asset} is not on {network}")))?;
    match network {
        "SOL" => spl_send(seed, contract, to, amount as u64).await,
        "ETH" => erc20_send(seed, contract, to, amount).await,
        "TRON" => tron_send_trc20(seed, contract, to, amount).await,
        other => Err(WalletError::Unsupported(format!("unknown network {other}"))),
    }
}

fn validate_recipient(network: &str, to: &str) -> Result<()> {
    match network {
        "SOL" => chains::sol_tx::parse_address(to).map(|_| ()),
        "ETH" => chains::eth::parse_address(to).map(|_| ()),
        "TRON" => chains::tron_tx::parse_address(to).map(|_| ()),
        other => Err(WalletError::Unsupported(format!("unknown network {other}"))),
    }
}

#[tauri::command]
pub async fn send_preview(
    asset: String,
    to: String,
    amount_minor: String,
    amount_usd: Option<f64>,
    outpoints: Option<Vec<String>>,
    network: Option<String>,
    state: State<'_, AppState>,
) -> Result<SendQuote> {
    let amount = parse_minor(&amount_minor)?;

    let seed = seed_copy(&state)?;
    let creator_fee = creator_fee_minor(&asset, amount as u128, amount_usd, &to);

    if let Some(chain) = btc_chain(&asset) {
        let (_keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let change = home_script(&seed, chain)?;

        let fee_out = btc_fee_output(&asset, chain, creator_fee)?;
        let creator_fee = fee_out.as_ref().map(|o| o.value).unwrap_or(0);
        let fixed = btc_fixed_outputs(&to, chain, amount, fee_out)?;

        let plan = match &outpoints {
            Some(chosen) => {
                let picked = pick_outputs(&utxos, chosen)?;
                chains::btc_tx::plan_with_outputs(&picked, fixed, change, rate)?
            }
            None => chains::btc_tx::select_outputs(&utxos, fixed, change, rate)?,
        };

        return Ok(SendQuote {
            asset,
            to,
            amount_minor: amount.to_string(),
            fee_minor: plan.fee.to_string(),
            creator_fee_minor: creator_fee.to_string(),
            total_minor: (amount as u128 + plan.fee as u128 + creator_fee as u128).to_string(),

            simulated: false,
        });
    }

    if asset == "ETH" {
        let wei = parse_wei(&amount_minor)?;
        let (_keys, _address, balance, max_fee, _tip) = eth_context(&seed).await?;
        chains::eth::parse_address(&to)?;

        let per_tx_gas = max_fee * chains::eth::TRANSFER_GAS as u128;

        let gas = if creator_fee > 0 { per_tx_gas * 2 } else { per_tx_gas };
        let needed = wei + creator_fee + gas;
        if needed > balance {
            return Err(WalletError::Funds(format!(
                "This needs {needed} wei including up to {gas} of gas, but the account holds {balance}."
            )));
        }

        return Ok(SendQuote {
            asset,
            to,
            amount_minor: wei.to_string(),
            fee_minor: gas.to_string(),
            creator_fee_minor: creator_fee.to_string(),
            total_minor: needed.to_string(),
            simulated: false,
        });
    }

    if asset == "TRON" {
        chains::tron_tx::parse_address(&to)?;
        let address = chains::tron_tx::address(&seed)?;
        let balance = chains::rpc::tron_trx_balance(&address).await?;
        let creator = creator_fee as u64;
        let reserve = chains::tron_tx::FEE_RESERVE_SUN;

        let fee_room = reserve as u128 * if creator > 0 { 2 } else { 1 };
        let needed = amount as u128 + creator as u128 + fee_room;
        if needed > balance as u128 {
            return Err(WalletError::Funds(format!(
                "This needs about {needed} sun including the network fee, but the account holds {balance}."
            )));
        }
        return Ok(SendQuote {
            asset,
            to,
            amount_minor: amount.to_string(),
            fee_minor: reserve.to_string(),
            creator_fee_minor: creator.to_string(),
            total_minor: (amount as u128 + creator as u128).to_string(),
            simulated: false,
        });
    }

    if chains::tokens::is_token(&asset) {
        let net = token_network(&asset, &network)?;
        validate_recipient(&net, to.trim())?;
        let held = token_balance(&seed, &asset, &net).await?;
        if amount as u128 > held {
            return Err(WalletError::Funds(format!(
                "This account holds {held} of {asset} on {net}, less than {amount}."
            )));
        }

        return Ok(SendQuote {
            asset,
            to,
            amount_minor: amount.to_string(),
            fee_minor: "0".into(),
            creator_fee_minor: "0".into(),
            total_minor: amount.to_string(),
            simulated: net == "SOL",
        });
    }

    if asset != "SOL" {
        return Err(WalletError::Unsupported(format!(
            "Sending {asset} is not implemented yet."
        )));
    }

    let creator_fee = creator_fee as u64;

    let network_fee = check_affordable(&seed, amount + creator_fee).await?;
    let blockhash = chains::rpc::sol_latest_blockhash().await?;
    let fee_to = donation_for("SOL").unwrap_or_default();
    let tx = chains::sol_tx::signed_transfer_with_fee(
        &seed, &to, amount, &fee_to, creator_fee, &blockhash,
    )?;
    chains::rpc::sol_simulate(&tx).await?;

    Ok(SendQuote {
        asset,
        to,
        amount_minor: amount.to_string(),
        fee_minor: network_fee.to_string(),
        creator_fee_minor: creator_fee.to_string(),
        total_minor: (amount as u128 + network_fee as u128 + creator_fee as u128).to_string(),
        simulated: true,
    })
}

fn btc_fee_output(
    asset: &str,
    chain: chains::btc_tx::Chain,
    creator_fee: u128,
) -> Result<Option<chains::btc_tx::Output>> {
    if creator_fee < chains::btc_tx::DUST as u128 {
        return Ok(None);
    }
    let addr = donation_for(asset).ok_or_else(|| {
        WalletError::Unsupported(format!("no donation address for {asset}"))
    })?;
    Ok(Some(chains::btc_tx::Output {
        script: chains::btc_tx::script_pubkey_for(&addr, chain)?,
        value: creator_fee as u64,
    }))
}

fn btc_fixed_outputs(
    to: &str,
    chain: chains::btc_tx::Chain,
    amount: u64,
    fee_out: Option<chains::btc_tx::Output>,
) -> Result<Vec<chains::btc_tx::Output>> {
    if amount < chains::btc_tx::DUST {
        return Err(WalletError::Funds(format!(
            "{amount} is below the dust limit, so the network would reject it."
        )));
    }
    let mut outputs = vec![chains::btc_tx::Output {
        script: chains::btc_tx::script_pubkey_for(to, chain)?,
        value: amount,
    }];
    if let Some(fee) = fee_out {
        outputs.push(fee);
    }
    Ok(outputs)
}

#[tauri::command]
pub async fn send_execute(
    asset: String,
    to: String,
    amount_minor: String,
    amount_usd: Option<f64>,
    outpoints: Option<Vec<String>>,
    network: Option<String>,
    state: State<'_, AppState>,
) -> Result<String> {
    let amount = parse_minor(&amount_minor)?;
    guard_large_send(&state, &asset, amount, amount_usd).await?;

    let seed = seed_copy(&state)?;
    let creator_fee = creator_fee_minor(&asset, amount as u128, amount_usd, &to);

    if let Some(chain) = btc_chain(&asset) {

        let (keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let change = home_script(&seed, chain)?;
        let fee_out = btc_fee_output(&asset, chain, creator_fee)?;
        let fixed = btc_fixed_outputs(&to, chain, amount, fee_out)?;

        let plan = match &outpoints {
            Some(chosen) => {
                let picked = pick_outputs(&utxos, chosen)?;
                chains::btc_tx::plan_with_outputs(&picked, fixed, change, rate)?
            }
            None => chains::btc_tx::select_outputs(&utxos, fixed, change, rate)?,
        };

        let signed = chains::btc_tx::build_signed(&keyring, &plan.inputs, &plan.outputs, 0)?;
        let apis = chains::rpc::btc_apis(chain);
        let broadcast = chains::rpc::esplora_broadcast(apis, &signed).await?;

        return Ok(if broadcast.len() == 64 {
            broadcast
        } else {
            chains::btc_tx::txid(&signed)
        });
    }

    if asset == "ETH" {
        let wei = parse_wei(&amount_minor)?;
        let (keys, address, balance, max_fee, tip) = eth_context(&seed).await?;

        let per_tx_gas = max_fee * chains::eth::TRANSFER_GAS as u128;
        let gas = if creator_fee > 0 { per_tx_gas * 2 } else { per_tx_gas };
        if wei + creator_fee + gas > balance {
            return Err(WalletError::Funds(format!(
                "This needs {} wei including gas, but the account holds {balance}.",
                wei + creator_fee + gas
            )));
        }

        let nonce = chains::rpc::eth_nonce(&address).await?;
        let main = chains::eth::Transfer {
            nonce,
            max_priority_fee: tip,
            max_fee,
            gas_limit: chains::eth::TRANSFER_GAS,
            to: chains::eth::parse_address(&to)?,
            value: wei,
            data: Vec::new(),
        };
        let signed = chains::eth::sign(&keys, &main)?;
        let id = chains::rpc::eth_broadcast(&signed).await?;

        if creator_fee > 0 {
            if let Some(fee_to) = donation_for("ETH") {
                if let Ok(fee_addr) = chains::eth::parse_address(&fee_to) {
                    let fee_tx = chains::eth::Transfer {
                        nonce: nonce + 1,
                        max_priority_fee: tip,
                        max_fee,
                        gas_limit: chains::eth::TRANSFER_GAS,
                        to: fee_addr,
                        value: creator_fee,
                        data: Vec::new(),
                    };
                    if let Ok(fee_signed) = chains::eth::sign(&keys, &fee_tx) {
                        let _ = chains::rpc::eth_broadcast(&fee_signed).await;
                    }
                }
            }
        }
        return Ok(id);
    }

    if asset == "TRON" {
        let creator = creator_fee as u64;
        let id = tron_send_trx(&seed, &to, amount).await?;

        if creator > 0 {
            if let Some(fee_to) = donation_for("TRON") {
                let _ = tron_send_trx(&seed, &fee_to, creator).await;
            }
        }
        return Ok(id);
    }

    if chains::tokens::is_token(&asset) {
        let net = token_network(&asset, &network)?;

        return token_send(&seed, &asset, &net, &to, amount as u128).await;
    }

    if asset != "SOL" {
        return Err(WalletError::Unsupported(format!(
            "Sending {asset} is not implemented yet."
        )));
    }

    let creator_fee = creator_fee as u64;
    check_affordable(&seed, amount + creator_fee).await?;

    let blockhash = chains::rpc::sol_latest_blockhash().await?;
    let fee_to = donation_for("SOL").unwrap_or_default();
    let tx = chains::sol_tx::signed_transfer_with_fee(
        &seed, &to, amount, &fee_to, creator_fee, &blockhash,
    )?;
    chains::rpc::sol_simulate(&tx).await?;

    chains::rpc::sol_broadcast(&tx).await
}

#[tauri::command]
pub fn forget_wallet(state: State<AppState>) -> Result<()> {
    state.wipe();

    match std::fs::remove_file(&state.vault_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(WalletError::Storage(e.to_string())),
    }

    keychain::forget()?;
    crate::appconfig::discard(&state.data_dir);
    Ok(())
}

#[tauri::command]
pub async fn fetch_activity(state: State<'_, AppState>) -> Result<Vec<chains::history::Entry>> {
    let seed = seed_copy(&state)?;
    let addresses: Vec<_> = chains::addresses(seed.as_ref())?
        .into_iter()
        .filter(|a| a.asset != "BTC" && a.asset != "LTC")
        .collect();

    let mut legacy = Vec::new();
    for (asset, chain) in [("BTC", chains::btc_tx::Chain::Bitcoin), ("LTC", chains::btc_tx::Chain::Litecoin)] {
        for kind in chains::btc_tx::Kind::ALL {
            for change in [false, true] {
                let keys = chains::btc_tx::keys_on(&seed, chain, kind, change, 0)?;
                legacy.push((asset, chains::btc_tx::address_of(&keys, chain)?));
            }
        }
    }

    let (mut entries, extra) = tokio::join!(
        chains::history::all(&addresses),
        futures::future::join_all(
            legacy.iter().map(|(asset, address)| chains::history::utxo_history(asset, address))
        ),
    );
    for found in extra.into_iter().flatten() {
        entries.extend(found);
    }
    Ok(chains::history::merge(entries))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxoEntry {
    pub txid: String,
    pub vout: u32,
    pub value_minor: String,

    pub key_index: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxoState {
    pub asset: String,
    pub total_minor: String,
    pub outputs: Vec<UtxoEntry>,

    pub sources: usize,
    pub fee_rate: f64,

    pub max_pieces: u32,
    pub dust_minor: String,
}

fn require_utxo_chain(asset: &str) -> Result<chains::btc_tx::Chain> {
    btc_chain(asset).ok_or_else(|| {
        WalletError::Unsupported(format!(
            "{asset} does not use outputs, so there is nothing to fragment.              Only Bitcoin and Litecoin do."
        ))
    })
}

#[tauri::command]
pub async fn utxo_state(asset: String, state: State<'_, AppState>) -> Result<UtxoState> {
    let chain = require_utxo_chain(&asset)?;
    let seed = seed_copy(&state)?;
    let (_keyring, utxos, fee_rate) = btc_context(&seed, chain).await?;

    let total: u64 = utxos.iter().map(|u| u.value).sum();
    let mut indices: Vec<u32> = utxos.iter().map(|u| u.key_index).collect();
    indices.sort_unstable();
    indices.dedup();

    let outputs = utxos
        .iter()
        .map(|u| UtxoEntry {

            txid: u.txid.iter().rev().map(|b| format!("{b:02x}")).collect(),
            vout: u.vout,
            value_minor: u.value.to_string(),
            key_index: u.key_index,
        })
        .collect();

    Ok(UtxoState {
        asset,
        total_minor: total.to_string(),
        sources: indices.len(),
        fee_rate,
        max_pieces: chains::btc_tx::max_pieces(total, utxos.len()),
        dust_minor: chains::btc_tx::DUST.to_string(),
        outputs,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FragmentQuote {
    pub asset: String,
    pub pieces: u32,
    pub per_piece_minor: String,
    pub amount_minor: String,
    pub fee_minor: String,

    pub change_minor: String,

    pub inputs_used: usize,

    pub sources: usize,
}

async fn fragment_plan(
    seed: &[u8; 64],
    chain: chains::btc_tx::Chain,
    amount: u64,
    pieces: u32,
) -> Result<(Vec<chains::btc_tx::Keys>, chains::btc_tx::FragmentPlan)> {
    let (keyring, utxos, rate) = btc_context(seed, chain).await?;

    let kind = chains::btc_tx::preferred(chain);
    let mut piece_scripts: Vec<Vec<u8>> = Vec::with_capacity(pieces as usize);
    for index in 1..=pieces {
        let keys = chains::btc_tx::keys_on(seed, chain, kind, false, index)?;
        piece_scripts.push(chains::btc_tx::script_of(&keys, chain)?);
    }
    let change = home_script(seed, chain)?;

    let plan =
        chains::btc_tx::fragment(&utxos, amount, pieces, &piece_scripts, change, rate)?;
    Ok((keyring, plan))
}

#[tauri::command]
pub async fn fragment_preview(
    asset: String,
    amount_minor: String,
    pieces: u32,
    state: State<'_, AppState>,
) -> Result<FragmentQuote> {
    let chain = require_utxo_chain(&asset)?;
    let amount = parse_minor(&amount_minor)?;
    let seed = seed_copy(&state)?;

    let (_keyring, plan) = fragment_plan(&seed, chain, amount, pieces).await?;

    Ok(FragmentQuote {
        asset,
        pieces: plan.pieces,
        per_piece_minor: plan.per_piece.to_string(),
        amount_minor: amount.to_string(),
        fee_minor: plan.fee.to_string(),
        change_minor: plan.change.to_string(),
        inputs_used: plan.inputs.len(),
        sources: plan.sources,
    })
}

#[tauri::command]
pub async fn fragment_execute(
    asset: String,
    amount_minor: String,
    pieces: u32,
    state: State<'_, AppState>,
) -> Result<String> {
    let chain = require_utxo_chain(&asset)?;
    let amount = parse_minor(&amount_minor)?;
    guard_large_send(&state, &asset, amount, None).await?;

    let seed = seed_copy(&state)?;

    let (keyring, plan) = fragment_plan(&seed, chain, amount, pieces).await?;

    let signed = chains::btc_tx::build_signed(&keyring, &plan.inputs, &plan.outputs, 0)?;
    let apis = chains::rpc::btc_apis(chain);
    let broadcast = chains::rpc::esplora_broadcast(apis, &signed).await?;

    Ok(if broadcast.len() == 64 {
        broadcast
    } else {
        chains::btc_tx::txid(&signed)
    })
}

#[tauri::command]
pub async fn consolidate_preview(asset: String, state: State<'_, AppState>) -> Result<SendQuote> {
    let chain = require_utxo_chain(&asset)?;
    let seed = seed_copy(&state)?;
    let (_keyring, utxos, rate) = btc_context(&seed, chain).await?;

    let dest = home_script(&seed, chain)?;
    let plan = chains::btc_tx::consolidate(&utxos, dest, rate)?;
    let value = plan.outputs[0].value;

    Ok(SendQuote {
        asset,
        to: chains::btc_tx::address_of(&home_keys(&seed, chain)?, chain)?,
        amount_minor: value.to_string(),
        fee_minor: plan.fee.to_string(),

        creator_fee_minor: "0".to_string(),
        total_minor: (value as u128 + plan.fee as u128).to_string(),
        simulated: false,
    })
}

#[tauri::command]
pub async fn consolidate_execute(asset: String, state: State<'_, AppState>) -> Result<String> {
    let chain = require_utxo_chain(&asset)?;
    let seed = seed_copy(&state)?;
    let (keyring, utxos, rate) = btc_context(&seed, chain).await?;

    let swept: u64 = utxos.iter().map(|u| u.value).sum();
    guard_large_send(&state, &asset, swept, None).await?;

    let dest = home_script(&seed, chain)?;
    let plan = chains::btc_tx::consolidate(&utxos, dest, rate)?;

    let signed = chains::btc_tx::build_signed(&keyring, &plan.inputs, &plan.outputs, 0)?;
    let apis = chains::rpc::btc_apis(chain);
    let broadcast = chains::rpc::esplora_broadcast(apis, &signed).await?;

    Ok(if broadcast.len() == 64 {
        broadcast
    } else {
        chains::btc_tx::txid(&signed)
    })
}

fn pick_outputs(
    available: &[chains::btc_tx::Utxo],
    chosen: &[String],
) -> Result<Vec<chains::btc_tx::Utxo>> {
    let mut picked = Vec::with_capacity(chosen.len());

    for want in chosen {
        let (txid, vout) = want
            .split_once(':')
            .ok_or_else(|| WalletError::Funds(format!("malformed coin reference: {want}")))?;
        let vout: u32 = vout
            .parse()
            .map_err(|_| WalletError::Funds(format!("malformed coin reference: {want}")))?;

        let found = available.iter().find(|u| {
            u.vout == vout
                && u.txid.iter().rev().map(|b| format!("{b:02x}")).collect::<String>() == txid
        });

        match found {
            Some(u) => picked.push(u.clone()),

            None => {
                return Err(WalletError::Funds(format!(
                    "One of the chosen coins is no longer available ({want}). Refresh and pick again."
                )))
            }
        }
    }

    Ok(picked)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Spendable {
    pub outpoint: String,
    pub value_minor: String,
    pub key_index: u32,
    pub address: String,
}

#[tauri::command]
pub async fn list_spendable(asset: String, state: State<'_, AppState>) -> Result<Vec<Spendable>> {
    let chain = require_utxo_chain(&asset)?;
    let seed = seed_copy(&state)?;
    let (keyring, utxos, _rate) = btc_context(&seed, chain).await?;

    utxos
        .iter()
        .map(|u| {
            let owner = keyring
                .get(u.key_index as usize)
                .ok_or_else(|| WalletError::Storage("missing key for an output".into()))?;
            Ok(Spendable {
                outpoint: format!(
                    "{}:{}",
                    u.txid.iter().rev().map(|b| format!("{b:02x}")).collect::<String>(),
                    u.vout
                ),
                value_minor: u.value.to_string(),
                key_index: u.key_index,
                address: chains::btc_tx::address_of(owner, chain)?,
            })
        })
        .collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveAddress {
    pub asset: String,
    pub address: String,
    pub index: u32,

    pub rotates: bool,
}

#[tauri::command]
pub async fn next_receive_address(
    asset: String,
    state: State<'_, AppState>,
) -> Result<ReceiveAddress> {
    let seed = seed_copy(&state)?;

    let chain = match btc_chain(&asset) {
        Some(chain) => chain,
        None => {
            let addresses = chains::addresses(&seed)?;
            let entry = addresses
                .into_iter()
                .find(|a| a.asset == asset)
                .ok_or_else(|| WalletError::Unsupported(format!("unknown asset {asset}")))?;
            let address = entry.address.ok_or_else(|| {
                WalletError::Unsupported(
                    entry.unsupported.unwrap_or_else(|| "no address".into()),
                )
            })?;
            return Ok(ReceiveAddress { asset, address, index: 0, rotates: false });
        }
    };

    let apis = chains::rpc::btc_apis(chain);

    if chains::btc_tx::preferred(chain) != chains::btc_tx::Kind::Segwit {
        let address = chains::btc_tx::address_of(&home_keys(&seed, chain)?, chain)?;
        return Ok(ReceiveAddress { asset, address, index: 0, rotates: false });
    }

    for index in 0..chains::rpc::SCAN_CEILING {
        let keys = chains::btc_tx::keys_at(&seed, chain, index)?;
        let address = chains::btc_tx::own_address(&keys.pubkey_hash, chain)?;

        if !chains::rpc::esplora_address_used(apis, &address).await? {
            return Ok(ReceiveAddress { asset, address, index, rotates: true });
        }
    }

    Err(WalletError::Unsupported(
        "every address in the scan range has been used already".into(),
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoneroKeys {
    pub address: String,
    pub spend_key: String,
    pub view_key: String,
    pub restore_height_hint: String,
}

#[tauri::command]
pub async fn reveal_monero_keys(state: State<'_, AppState>) -> Result<MoneroKeys> {
    require_2fa(&state)?;
    let seed = seed_copy(&state)?;
    let keys = chains::xmr::keys(&seed)?;

    let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();

    let out = MoneroKeys {
        address: chains::xmr::address(&seed)?,
        spend_key: hex(keys.spend.to_bytes()),
        view_key: hex(keys.view.to_bytes()),

        restore_height_hint: "the block height when you first received Monero here".into(),
    };

    Ok(out)
}

#[tauri::command]
pub async fn xmr_status(
    endpoint: String,
    state: State<'_, AppState>,
) -> Result<chains::xmr_rpc::WalletStatus> {
    let seed = seed_copy(&state)?;
    let expected = chains::xmr::address(&seed)?;
    chains::xmr_rpc::status(&endpoint, &expected).await
}

#[tauri::command]
pub async fn xmr_balance(endpoint: String) -> Result<chains::xmr_rpc::Balance> {
    chains::xmr_rpc::balance(&endpoint).await
}

#[tauri::command]
pub async fn xmr_preview(
    endpoint: String,
    to: String,
    amount_minor: String,
    amount_usd: Option<f64>,
) -> Result<chains::xmr_rpc::Transfer> {
    let amount = parse_minor(&amount_minor)?;
    let fee = creator_fee_minor("XMR", amount as u128, amount_usd, &to) as u64;
    let fee_to = donation_for("XMR");
    chains::xmr_rpc::estimate(&endpoint, &to, amount, fee_to.as_deref(), fee).await
}

#[tauri::command]
pub async fn xmr_send(
    endpoint: String,
    to: String,
    amount_minor: String,
    amount_usd: Option<f64>,
    state: State<'_, AppState>,
) -> Result<chains::xmr_rpc::Transfer> {
    let amount = parse_minor(&amount_minor)?;
    guard_large_send(&state, "XMR", amount, amount_usd).await?;

    let fee = creator_fee_minor("XMR", amount as u128, amount_usd, &to) as u64;
    let fee_to = donation_for("XMR");
    chains::xmr_rpc::send(&endpoint, &to, amount, fee_to.as_deref(), fee).await
}

#[tauri::command]
pub fn inactivity_check(state: State<AppState>) -> crate::inactivity::Status {
    match crate::inactivity::check(&state.data_dir, &state.vault_path) {
        crate::inactivity::Outcome::Idle(s)
        | crate::inactivity::Outcome::Wiped(s)
        | crate::inactivity::Outcome::SweepDue(s) => s,
    }
}

#[tauri::command]
pub fn inactivity_set_months(
    months: u32,
    state: State<AppState>,
) -> Result<crate::inactivity::Status> {
    crate::inactivity::set_months(&state.data_dir, months)
}

#[tauri::command]
pub fn inactivity_set_action(
    action: String,
    state: State<AppState>,
) -> Result<crate::inactivity::Status> {
    let action = match action.as_str() {
        "donate" => crate::inactivity::Action::Donate,
        "delete" => crate::inactivity::Action::Delete,
        other => {
            return Err(WalletError::Unsupported(format!("unknown action {other}")))
        }
    };

    if matches!(action, crate::inactivity::Action::Donate)
        && store::lock_info(&state.vault_path)
            .map(|i| i.needs_passphrase)
            .unwrap_or(false)
    {
        return Err(WalletError::Unsupported(
            "Send to donations cannot be used while a passphrase is set, because an \
             unattended sweep cannot ask for one. Remove the passphrase first, or choose \
             delete."
                .into(),
        ));
    }

    crate::inactivity::set_action(&state.data_dir, action)
}

#[tauri::command]
pub fn set_vault_passphrase(
    current: Option<String>,
    next: Option<String>,
    state: State<AppState>,
) -> Result<()> {

    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    if !store::exists(&state.vault_path) {
        return Err(WalletError::NoVault);
    }

    let key = keychain::load()?;

    let needs = store::lock_info(&state.vault_path)
        .map(|i| i.needs_passphrase)
        .unwrap_or(false);
    let current = current.as_deref().filter(|p| !p.is_empty());
    let read_pass = if needs { current } else { None };

    let payload = store::read(&state.vault_path, &key, read_pass)?;

    let next = next.as_deref().filter(|p| !p.is_empty());
    store::write(&state.vault_path, &key, next, &payload)?;

    store::read(&state.vault_path, &key, next)?;

    if next.is_some() {
        let _ = crate::inactivity::set_action(&state.data_dir, crate::inactivity::Action::Delete);
        if let Ok(mut cfg) = crate::appconfig::load(&state.data_dir) {
            if cfg.stay_signed_in {
                cfg.stay_signed_in = false;
                cfg.sign_in_paused = false;
                let _ = crate::appconfig::save(&state.data_dir, &cfg);
            }
        }
    }

    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepResult {
    pub asset: String,

    pub txid: Option<String>,

    pub skipped: Option<String>,

    pub error: Option<String>,
}

impl SweepResult {
    fn sent(asset: &str, txid: String) -> Self {
        Self { asset: asset.into(), txid: Some(txid), skipped: None, error: None }
    }
    fn skip(asset: &str, why: &str) -> Self {
        Self { asset: asset.into(), txid: None, skipped: Some(why.into()), error: None }
    }
    fn failed(asset: &str, e: impl std::fmt::Display) -> Self {
        Self { asset: asset.into(), txid: None, skipped: None, error: Some(e.to_string()) }
    }
}

fn recover_seed(state: &State<AppState>) -> Result<[u8; 64]> {
    let key = keychain::load()?;

    let payload = store::read(&state.vault_path, &key, None)?;
    let parsed = seed::parse(&payload.mnemonic)?;
    Ok(*seed::to_seed(&parsed))
}

fn donation_for(asset: &str) -> Option<String> {
    crate::donation::addresses()
        .into_iter()
        .find(|d| d.asset == asset)
        .map(|d| d.address)
}

async fn sweep_all(seed: &[u8; 64]) -> Vec<SweepResult> {
    let mut out = Vec::new();

    for (asset, chain) in [
        ("BTC", chains::btc_tx::Chain::Bitcoin),
        ("LTC", chains::btc_tx::Chain::Litecoin),
    ] {
        let Some(to) = donation_for(asset) else { continue };
        out.push(match btc_context(seed, chain).await {
            Ok((keyring, utxos, rate)) => {
                if utxos.is_empty() {
                    SweepResult::skip(asset, "no confirmed balance")
                } else {
                    match chains::btc_tx::script_pubkey_for(&to, chain)
                        .and_then(|dest| chains::btc_tx::consolidate(&utxos, dest, rate))
                        .and_then(|plan| {
                            chains::btc_tx::build_signed(&keyring, &plan.inputs, &plan.outputs, 0)
                        }) {
                        Ok(signed) => {
                            match chains::rpc::esplora_broadcast(
                                chains::rpc::btc_apis(chain),
                                &signed,
                            )
                            .await
                            {
                                Ok(id) => SweepResult::sent(asset, id),
                                Err(e) => SweepResult::failed(asset, e),
                            }
                        }
                        Err(e) => SweepResult::failed(asset, e),
                    }
                }
            }
            Err(e) => SweepResult::failed(asset, e),
        });
    }

    if let Some(to) = donation_for("ETH") {
        out.push(match eth_context(seed).await {
            Ok((keys, address, balance, max_fee, tip)) => {
                let cost = max_fee * chains::eth::TRANSFER_GAS as u128;
                if balance <= cost {
                    SweepResult::skip("ETH", "balance does not cover gas")
                } else {
                    match chains::eth::parse_address(&to) {
                        Ok(to) => match chains::rpc::eth_nonce(&address).await {
                            Ok(nonce) => {
                                let tx = chains::eth::Transfer {
                                    nonce,
                                    max_priority_fee: tip,
                                    max_fee,
                                    gas_limit: chains::eth::TRANSFER_GAS,
                                    to,
                                    value: balance - cost,
                                    data: Vec::new(),
                                };
                                match chains::eth::sign(&keys, &tx) {
                                    Ok(signed) => {
                                        match chains::rpc::eth_broadcast(&signed).await {
                                            Ok(id) => SweepResult::sent("ETH", id),
                                            Err(e) => SweepResult::failed("ETH", e),
                                        }
                                    }
                                    Err(e) => SweepResult::failed("ETH", e),
                                }
                            }
                            Err(e) => SweepResult::failed("ETH", e),
                        },
                        Err(e) => SweepResult::failed("ETH", e),
                    }
                }
            }
            Err(e) => SweepResult::failed("ETH", e),
        });
    }

    if let Some(to) = donation_for("SOL") {
        let from = chains::sol_address(seed, chains::sol_exodus()).unwrap_or_default();
        out.push(match chains::rpc::sol_balance_of(&from).await {
            Ok(balance) => {
                let balance = balance.max(0) as u64;
                let fee = chains::sol_tx::LAMPORTS_PER_SIGNATURE;
                if balance <= fee {
                    SweepResult::skip("SOL", "balance does not cover the fee")
                } else {
                    match chains::rpc::sol_latest_blockhash().await {
                        Ok(blockhash) => {
                            match chains::sol_tx::signed_transfer(
                                seed,
                                &to,
                                balance - fee,
                                &blockhash,
                            ) {
                                Ok(tx) => match chains::rpc::sol_broadcast(&tx).await {
                                    Ok(id) => SweepResult::sent("SOL", id),
                                    Err(e) => SweepResult::failed("SOL", e),
                                },
                                Err(e) => SweepResult::failed("SOL", e),
                            }
                        }
                        Err(e) => SweepResult::failed("SOL", e),
                    }
                }
            }
            Err(e) => SweepResult::failed("SOL", e),
        });
    }

    out.push(SweepResult::skip("TRON", "sending Tron is not implemented"));
    out.push(SweepResult::skip("USDT", "sending Tether is not implemented"));
    out.push(SweepResult::skip("USDC", "sending USD Coin is not implemented"));
    out.push(SweepResult::skip(
        "XMR",
        "needs the Monero daemon, which only runs while signed in",
    ));

    out
}

#[tauri::command]
pub async fn inactivity_sweep(state: State<'_, AppState>) -> Result<Vec<SweepResult>> {
    match crate::inactivity::check(&state.data_dir, &state.vault_path) {
        crate::inactivity::Outcome::SweepDue(_) => {}
        _ => {
            return Err(WalletError::Unsupported(
                "no inactivity sweep is due".into(),
            ))
        }
    }

    let seed = recover_seed(&state)?;
    let results = sweep_all(&seed).await;

    let all_ok = results.iter().all(|r| r.error.is_none());
    crate::inactivity::finish_sweep(&state.data_dir, &state.vault_path, all_ok);

    Ok(results)
}

#[tauri::command]
pub fn donation_addresses() -> Vec<crate::donation::Donation> {
    crate::donation::addresses()
}

#[tauri::command]
pub fn tor_state(state: State<AppState>) -> crate::tor::TorState {
    crate::tor::state(&state.data_dir, state.tor_running())
}

#[tauri::command]
pub async fn tor_start(state: State<'_, AppState>) -> Result<crate::tor::TorState> {
    let data_dir = state.data_dir.clone();

    crate::tor::install(&data_dir).await?;

    if crate::tor::proxy_alive().await {
        crate::tor::set_routing(true);
        return Ok(crate::tor::state(&data_dir, state.tor_running()));
    }

    state.stop_tor();
    let child = crate::tor::spawn(&data_dir)?;
    {
        let mut guard = state
            .tor
            .lock()
            .map_err(|_| WalletError::Storage("tor lock poisoned".into()))?;
        *guard = Some(child);
    }

    crate::tor::wait_until_ready().await?;
    crate::tor::set_routing(true);

    Ok(crate::tor::state(&data_dir, state.tor_running()))
}

#[tauri::command]
pub fn tor_stop(state: State<AppState>) -> crate::tor::TorState {
    crate::tor::set_routing(false);
    state.stop_tor();
    crate::tor::state(&state.data_dir, false)
}

#[tauri::command]
pub async fn monero_setup_state(
    state: State<'_, AppState>,
) -> Result<chains::xmr_setup::SetupState> {
    let data_dir = state.data_dir.clone();
    let endpoint = format!("http://127.0.0.1:{}/json_rpc", chains::xmr_setup::RPC_PORT);
    let live = chains::xmr_rpc::ping(&endpoint).await.is_ok();
    Ok(chains::xmr_setup::state(&data_dir, live))
}

#[tauri::command]
pub async fn monero_setup_run(
    daemon: Option<String>,
    state: State<'_, AppState>,
) -> Result<chains::xmr_setup::SetupState> {
    let data_dir = state.data_dir.clone();
    let daemon = daemon
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| chains::xmr_setup::DEFAULT_DAEMON.to_string());

    let seed = seed_copy(&state)?;
    let keys = chains::xmr::keys(&seed)?;
    let address = chains::xmr::address(&seed)?;
    let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let spend = hex(keys.spend.to_bytes());
    let view = hex(keys.view.to_bytes());

    chains::xmr_setup::install(&data_dir).await?;

    state.stop_monero();

    let endpoint = format!("http://127.0.0.1:{}/json_rpc", chains::xmr_setup::RPC_PORT);
    let adopted = chains::xmr_setup::ensure_running(&data_dir, &daemon, &endpoint).await?;

    if let Some(child) = adopted {
        let mut guard = state
            .monero
            .lock()
            .map_err(|_| WalletError::Storage("monero lock poisoned".into()))?;
        *guard = Some(child);
    }

    let password = chains::xmr_setup::wallet_password()?;
    chains::xmr_setup::ensure_wallet(
        &data_dir, &endpoint, &daemon, &address, &spend, &view, &password,
    )
    .await?;

    chains::xmr_setup::write_readme(&data_dir);

    Ok(chains::xmr_setup::state(&data_dir, true))
}

#[tauri::command]
pub async fn monero_stop(
    state: State<'_, AppState>,
) -> Result<chains::xmr_setup::SetupState> {
    let data_dir = state.data_dir.clone();
    let endpoint = format!("http://127.0.0.1:{}/json_rpc", chains::xmr_setup::RPC_PORT);

    let _ = chains::xmr_rpc::close_wallet(&endpoint).await;

    state.stop_monero();
    chains::xmr_setup::stop_any(&data_dir);
    Ok(chains::xmr_setup::state(&data_dir, false))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorState {
    pub enabled: bool,

    pub pass_valid: bool,
}

#[tauri::command]
pub fn two_factor_state(state: State<AppState>) -> Result<TwoFactorState> {
    let enabled = crate::appconfig::load(&state.data_dir)?.two_factor;
    let pass_valid = state
        .two_factor
        .lock()
        .map(|tf| tf.pass_expiry.map(crate::twofa::pass_valid).unwrap_or(false))
        .unwrap_or(false);
    Ok(TwoFactorState { enabled, pass_valid })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpSetup {

    pub secret: String,

    pub uri: String,
}

#[tauri::command]
pub fn begin_totp_setup(state: State<AppState>) -> Result<TotpSetup> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    let secret = crate::totp::new_secret();
    let uri = crate::totp::provisioning_uri(&secret, "wallet");

    let mut tf = state
        .two_factor
        .lock()
        .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;
    tf.pending_secret = Some(secret.clone());

    Ok(TotpSetup { secret, uri })
}

#[tauri::command]
pub fn confirm_totp(code: String, state: State<AppState>) -> Result<bool> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    let pending = {
        let tf = state
            .two_factor
            .lock()
            .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;
        tf.pending_secret.clone()
    };
    let secret = pending.ok_or_else(|| {
        WalletError::Unsupported("Start two-factor setup before confirming a code.".into())
    })?;

    if !crate::totp::verify(&secret, &code)? {
        return Ok(false);
    }

    let mut cfg = crate::appconfig::load(&state.data_dir)?;
    cfg.two_factor = true;
    cfg.totp_secret = secret;
    crate::appconfig::save(&state.data_dir, &cfg)?;

    if let Ok(mut tf) = state.two_factor.lock() {
        tf.pending_secret = None;

        tf.pass_expiry = Some(crate::twofa::pass_expires_at());
        tf.wrong = 0;
        tf.locked_until = None;
    }
    Ok(true)
}

#[tauri::command]
pub fn disable_two_factor(state: State<AppState>) -> Result<()> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    let mut cfg = crate::appconfig::load(&state.data_dir)?;
    cfg.two_factor = false;
    cfg.totp_secret = String::new();
    crate::appconfig::save(&state.data_dir, &cfg)?;

    if let Ok(mut tf) = state.two_factor.lock() {
        *tf = crate::session::TwoFactor::default();
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResult {

    pub status: String,

    pub remaining: Option<u32>,

    pub locked_until: Option<i64>,
}

#[tauri::command]
pub fn verify_2fa(code: String, state: State<AppState>) -> Result<VerifyResult> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    let secret = crate::appconfig::load(&state.data_dir)?.totp_secret;
    if secret.is_empty() {
        return Ok(VerifyResult { status: "none".into(), remaining: None, locked_until: None });
    }

    let now = crate::now_unix();
    let mut tf = state
        .two_factor
        .lock()
        .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;

    if crate::twofa::locked(tf.locked_until, now) {
        return Ok(VerifyResult {
            status: "lockedOut".into(),
            remaining: None,
            locked_until: tf.locked_until,
        });
    }

    if crate::totp::verify(&secret, &code)? {
        tf.wrong = 0;
        tf.locked_until = None;
        tf.pass_expiry = Some(crate::twofa::pass_expires_at());
        return Ok(VerifyResult { status: "ok".into(), remaining: None, locked_until: None });
    }

    tf.wrong += 1;
    if let Some(until) = crate::twofa::lock_after(tf.wrong, now) {
        tf.locked_until = Some(until);
        tf.wrong = 0;
        return Ok(VerifyResult {
            status: "lockedOut".into(),
            remaining: None,
            locked_until: Some(until),
        });
    }
    Ok(VerifyResult {
        status: "wrong".into(),
        remaining: Some(crate::twofa::LOCK_AT - tf.wrong),
        locked_until: None,
    })
}

#[tauri::command]
pub fn verify_2fa_seed(mnemonic: String, state: State<AppState>) -> Result<bool> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    let entered = Zeroizing::new(mnemonic);
    let parsed = seed::parse(&entered)?;

    let matches = {
        let guard = state
            .unlocked
            .lock()
            .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
        match guard.as_ref() {
            Some(session) => {
                ct_eq(parsed.to_string().as_bytes(), session.mnemonic.as_bytes())
            }
            None => return Err(WalletError::Locked),
        }
    };

    if matches {
        if let Ok(mut tf) = state.two_factor.lock() {
            tf.wrong = 0;
            tf.locked_until = None;
            tf.pass_expiry = Some(crate::twofa::pass_expires_at());
        }
    }
    Ok(matches)
}

fn require_step_up(state: &State<'_, AppState>, action: crate::twofa::Guarded) -> Result<()> {
    let cfg = crate::appconfig::load(&state.data_dir)?;
    let need = crate::twofa::required(&cfg, action);
    if !need.any() {
        return Ok(());
    }

    let mut tf = state
        .two_factor
        .lock()
        .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;
    let totp_ok = tf.pass_expiry.map(crate::twofa::pass_valid).unwrap_or(false);

    if tf.dormant_pending || (need.totp && !totp_ok) {
        return Err(WalletError::TwoFactorRequired);
    }
    if need.totp {
        tf.pass_expiry = None;
    }
    Ok(())
}

fn require_2fa(state: &State<AppState>) -> Result<()> {
    require_step_up(state, crate::twofa::Guarded::RevealSecret)
}

#[tauri::command]
pub async fn reveal_seed(state: State<'_, AppState>) -> Result<String> {
    require_2fa(&state)?;

    let mnemonic = {
        let guard = state
            .unlocked
            .lock()
            .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
        match guard.as_ref() {
            Some(session) => session.mnemonic.to_string(),
            None => return Err(WalletError::Locked),
        }
    };

    Ok(mnemonic)
}

fn swap_creds() -> Result<Zeroizing<String>> {
    let key = crate::swapcfg::api_key();
    if key.trim().is_empty() {
        return Err(WalletError::Unsupported(
            "Swaps are not available in this build: no ChangeNOW key was compiled in."
                .into(),
        ));
    }
    Ok(key)
}

fn own_address(seed: &[u8; 64], asset: &str) -> Result<String> {
    chains::addresses(seed)?
        .into_iter()
        .find(|a| a.asset == asset)
        .and_then(|a| a.address)
        .ok_or_else(|| WalletError::Unsupported(format!("no address for {asset}")))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapQuote {
    pub from: String,
    pub to: String,
    pub amount_from_minor: String,

    pub amount_to_minor: String,
    pub provider: String,
}

#[tauri::command]
pub async fn swap_quote(
    from: String,
    to: String,
    amount_minor: String,
) -> Result<SwapQuote> {
    let key = swap_creds()?;
    let dp_from = chains::swap::decimals(&from)
        .ok_or_else(|| WalletError::Unsupported(format!("{from} cannot be swapped")))?;
    let dp_to = chains::swap::decimals(&to)
        .ok_or_else(|| WalletError::Unsupported(format!("{to} cannot be swapped")))?;

    let amount_from = chains::swap::minor_to_decimal(&amount_minor, dp_from)?;
    let q = chains::swap::quote(&key, &from, &to, &amount_from).await?;
    let amount_to_minor = chains::swap::decimal_to_minor_floor(&q.amount_to, dp_to).unwrap_or(0);

    Ok(SwapQuote {
        from,
        to,
        amount_from_minor: amount_minor,
        amount_to_minor: amount_to_minor.to_string(),
        provider: q.provider,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapTrade {
    pub id: String,
    pub from: String,
    pub to: String,

    pub deposit_address: String,

    pub deposit_memo: String,

    pub deposit_amount_minor: String,

    pub payout_address: String,
    pub amount_to_minor: String,
    pub provider: String,
    pub status: String,
}

#[tauri::command]
pub async fn swap_create(
    from: String,
    to: String,
    amount_minor: String,
    state: State<'_, AppState>,
) -> Result<SwapTrade> {
    let key = swap_creds()?;
    let seed = seed_copy(&state)?;

    let dp_from = chains::swap::decimals(&from)
        .ok_or_else(|| WalletError::Unsupported(format!("{from} cannot be swapped")))?;
    let dp_to = chains::swap::decimals(&to)
        .ok_or_else(|| WalletError::Unsupported(format!("{to} cannot be swapped")))?;

    let amount_from = chains::swap::minor_to_decimal(&amount_minor, dp_from)?;
    let payout = own_address(&seed, &to)?;
    let refund = own_address(&seed, &from)?;

    let trade =
        chains::swap::create(&key, &from, &to, &amount_from, &payout, &refund).await?;

    let deposit_minor = chains::swap::decimal_to_minor(&trade.amount_from, dp_from)
        .unwrap_or_else(|_| amount_minor.parse::<u128>().unwrap_or(0));
    let amount_to_minor =
        chains::swap::decimal_to_minor_floor(&trade.amount_to, dp_to).unwrap_or(0);

    Ok(SwapTrade {
        id: trade.id,
        from,
        to,
        deposit_address: trade.deposit_address,
        deposit_memo: trade.deposit_memo,
        deposit_amount_minor: deposit_minor.to_string(),
        payout_address: if trade.payout_address.is_empty() {
            payout
        } else {
            trade.payout_address
        },
        amount_to_minor: amount_to_minor.to_string(),
        provider: trade.provider,
        status: trade.status,
    })
}

#[tauri::command]
pub async fn swap_fund(
    from: String,
    deposit_address: String,
    amount_minor: String,
    memo: Option<String>,
    endpoint: Option<String>,
    state: State<'_, AppState>,
) -> Result<String> {
    guard_large_send(&state, &from, parse_minor(&amount_minor)?, None).await?;

    if memo.as_deref().map(|m| !m.trim().is_empty()).unwrap_or(false) {
        return Err(WalletError::Unsupported(
            "This route needs a deposit memo this wallet cannot attach. Try a \
             different amount or pair."
                .into(),
        ));
    }

    let seed = seed_copy(&state)?;
    let to = deposit_address;

    if let Some(chain) = btc_chain(&from) {
        let amount = parse_minor(&amount_minor)?;
        let (keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let change = home_script(&seed, chain)?;
        let fixed = btc_fixed_outputs(&to, chain, amount, None)?;
        let plan = chains::btc_tx::select_outputs(&utxos, fixed, change, rate)?;
        let signed = chains::btc_tx::build_signed(&keyring, &plan.inputs, &plan.outputs, 0)?;
        let apis = chains::rpc::btc_apis(chain);
        let broadcast = chains::rpc::esplora_broadcast(apis, &signed).await?;
        return Ok(if broadcast.len() == 64 {
            broadcast
        } else {
            chains::btc_tx::txid(&signed)
        });
    }

    if from == "ETH" {
        let wei = parse_wei(&amount_minor)?;
        let (keys, address, balance, max_fee, tip) = eth_context(&seed).await?;
        let dest = chains::eth::parse_address(&to)?;
        let gas = max_fee * chains::eth::TRANSFER_GAS as u128;
        if wei + gas > balance {
            return Err(WalletError::Funds(format!(
                "This needs {} wei including gas, but the account holds {balance}.",
                wei + gas
            )));
        }
        let nonce = chains::rpc::eth_nonce(&address).await?;
        let tx = chains::eth::Transfer {
            nonce,
            max_priority_fee: tip,
            max_fee,
            gas_limit: chains::eth::TRANSFER_GAS,
            to: dest,
            value: wei,
            data: Vec::new(),
        };
        let signed = chains::eth::sign(&keys, &tx)?;
        return chains::rpc::eth_broadcast(&signed).await;
    }

    if from == "SOL" {
        let amount = parse_minor(&amount_minor)?;
        check_affordable(&seed, amount).await?;
        let blockhash = chains::rpc::sol_latest_blockhash().await?;
        let tx = chains::sol_tx::signed_transfer(&seed, &to, amount, &blockhash)?;
        chains::rpc::sol_simulate(&tx).await?;
        return chains::rpc::sol_broadcast(&tx).await;
    }

    if from == "XMR" {
        let amount = parse_minor(&amount_minor)?;
        let endpoint = endpoint.filter(|e| !e.trim().is_empty()).ok_or_else(|| {
            WalletError::Unsupported("Monero must be running to fund a swap from XMR.".into())
        })?;
        let transfer = chains::xmr_rpc::send(&endpoint, &to, amount, None, 0).await?;
        return Ok(transfer.tx_hash);
    }

    if from == "TRON" {
        let amount = parse_minor(&amount_minor)?;
        return tron_send_trx(&seed, &to, amount).await;
    }

    if from == "USDC" {
        let amount = parse_minor(&amount_minor)?;
        let mint = chains::tokens::contract("USDC", "SOL")
            .ok_or_else(|| WalletError::Unsupported("USDC mint missing".into()))?;
        return spl_send(&seed, mint, &to, amount).await;
    }

    if from == "USDT" {
        let amount = parse_minor(&amount_minor)?;
        let contract = chains::tokens::contract("USDT", "TRON")
            .ok_or_else(|| WalletError::Unsupported("USDT contract missing".into()))?;
        return tron_send_trc20(&seed, contract, &to, amount as u128).await;
    }

    Err(WalletError::Unsupported(format!(
        "Swapping from {from} is not supported: this wallet cannot send it yet."
    )))
}

#[tauri::command]
pub async fn swap_status(id: String) -> Result<String> {
    let key = swap_creds()?;
    chains::swap::status(&key, &id).await
}

fn transfer_usd(
    asset: &str,
    amount_minor: u64,
    prices: &HashMap<String, chains::rpc::Quote>,
) -> Option<f64> {
    let dp = chains::swap::decimals(asset)?;
    let quote = prices.get(asset)?;
    Some(amount_minor as f64 / 10f64.powi(dp as i32) * quote.price)
}

async fn guard_large_send(
    state: &State<'_, AppState>,
    asset: &str,
    amount_minor: u64,
    supplied_usd: Option<f64>,
) -> Result<()> {
    let config = crate::appconfig::load(&state.data_dir)?;
    if !config.step_up.large_send {
        return Ok(());
    }

    if !crate::twofa::required(&config, crate::twofa::Guarded::LargeSend).any() {
        return Ok(());
    }

    let prices = chains::rpc::prices("usd").await.ok();
    let value = prices
        .as_ref()
        .and_then(|p| transfer_usd(asset, amount_minor, p))
        .or_else(|| supplied_usd.filter(|v| v.is_finite() && *v >= 0.0));
    let total = match prices.as_ref() {
        Some(p) => portfolio_usd(state, p).await,
        None => None,
    };

    if crate::twofa::is_large_send(value, total) {
        require_step_up(state, crate::twofa::Guarded::LargeSend)?;
    }
    Ok(())
}

async fn portfolio_usd(
    state: &State<'_, AppState>,
    prices: &HashMap<String, chains::rpc::Quote>,
) -> Option<f64> {
    let addresses = {
        let guard = state.unlocked.lock().ok()?;
        chains::addresses(guard.as_ref()?.seed.as_ref()).ok()?
    };
    sum_balances(chains::rpc::balances(&addresses).await, prices)
}

fn sum_balances(
    balances: Vec<chains::rpc::AssetBalance>,
    prices: &HashMap<String, chains::rpc::Quote>,
) -> Option<f64> {
    let mut total = 0.0;
    let mut priced_anything = false;
    for balance in balances {

        if balance.network.is_some() {
            continue;
        }
        let (Some(minor), Some(dp)) = (
            balance.minor.as_deref(),
            chains::swap::decimals(&balance.asset),
        ) else {
            continue;
        };
        let (Ok(units), Some(quote)) = (minor.parse::<f64>(), prices.get(&balance.asset)) else {
            continue;
        };
        total += units / 10f64.powi(dp as i32) * quote.price;
        priced_anything = true;
    }

    priced_anything.then_some(total)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepUpView {
    pub dormant_days: u32,
    pub large_send: bool,

    pub totp_available: bool,

    pub large_send_usd: f64,
    pub large_send_share: f64,
}

fn step_up_view(data_dir: &std::path::Path) -> Result<StepUpView> {
    let cfg = crate::appconfig::load(data_dir)?;
    Ok(StepUpView {
        dormant_days: cfg.step_up.dormant_days,
        large_send: cfg.step_up.large_send,
        totp_available: cfg.two_factor && !cfg.totp_secret.is_empty(),
        large_send_usd: crate::appconfig::LARGE_SEND_USD,
        large_send_share: crate::appconfig::LARGE_SEND_SHARE,
    })
}

#[tauri::command]
pub fn step_up_settings(state: State<AppState>) -> Result<StepUpView> {
    step_up_view(&state.data_dir)
}

#[tauri::command]
pub fn set_step_up_settings(
    dormant_days: u32,
    large_send: bool,
    state: State<AppState>,
) -> Result<StepUpView> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    let mut cfg = crate::appconfig::load(&state.data_dir)?;

    cfg.step_up.dormant_days = dormant_days.min(365);
    cfg.step_up.large_send = large_send;
    crate::appconfig::save(&state.data_dir, &cfg)?;
    step_up_view(&state.data_dir)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub totp: bool,

    pub totp_done: bool,
}

fn guarded_from(action: &str) -> crate::twofa::Guarded {
    match action {
        "send" => crate::twofa::Guarded::LargeSend,
        "dormant" => crate::twofa::Guarded::Dormant,
        _ => crate::twofa::Guarded::RevealSecret,
    }
}

#[tauri::command]
pub fn step_up_challenge(action: String, state: State<AppState>) -> Result<Challenge> {
    let cfg = crate::appconfig::load(&state.data_dir)?;
    let need = crate::twofa::required(&cfg, guarded_from(&action));

    let tf = state
        .two_factor
        .lock()
        .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;
    Ok(Challenge {
        totp: need.totp,
        totp_done: tf.pass_expiry.map(crate::twofa::pass_valid).unwrap_or(false),
    })
}

#[tauri::command]
pub fn dormant_step_up_pending(state: State<AppState>) -> Result<bool> {
    let tf = state
        .two_factor
        .lock()
        .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;
    Ok(tf.dormant_pending)
}

#[tauri::command]
pub fn clear_dormant_step_up(state: State<AppState>) -> Result<bool> {
    let cfg = crate::appconfig::load(&state.data_dir)?;
    let need = crate::twofa::required(&cfg, crate::twofa::Guarded::Dormant);

    let mut tf = state
        .two_factor
        .lock()
        .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;
    if !tf.dormant_pending {
        return Ok(true);
    }

    let totp_ok = tf.pass_expiry.map(crate::twofa::pass_valid).unwrap_or(false);
    if need.totp && !totp_ok {
        return Ok(false);
    }

    if need.totp {
        tf.pass_expiry = None;
    }
    tf.dormant_pending = false;
    Ok(true)
}

#[tauri::command]
pub fn settings_unreadable(state: State<AppState>) -> Result<bool> {
    Ok(crate::appconfig::is_unreadable(&state.data_dir))
}

#[tauri::command]
pub fn reset_settings(state: State<AppState>) -> Result<()> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    crate::appconfig::reset(&state.data_dir)
}

#[cfg(test)]
mod gate_wiring {

    const SOURCE: &str = include_str!("commands.rs");

    fn body_of(name: &str) -> &'static str {
        let sig = format!("\npub async fn {name}(");
        let start = SOURCE
            .find(&sig)
            .unwrap_or_else(|| panic!("{name} is not in this file any more"));
        let rest = &SOURCE[start + sig.len()..];
        let end = rest.find("\npub ").unwrap_or(rest.len());
        &rest[..end]
    }

    const MOVES_MONEY: [&str; 5] = [
        "send_execute",
        "fragment_execute",
        "consolidate_execute",
        "xmr_send",
        "swap_fund",
    ];

    const ONLY_QUOTES: [&str; 3] = ["send_preview", "fragment_preview", "consolidate_preview"];

    #[test]
    fn every_path_that_moves_money_is_gated() {
        for name in MOVES_MONEY {
            assert!(
                body_of(name).contains("guard_large_send("),
                "{name} can move money without passing the large-send gate"
            );
        }
    }

    #[test]
    fn no_preview_is_gated() {
        for name in ONLY_QUOTES {
            assert!(
                !body_of(name).contains("guard_large_send("),
                "{name} only quotes, so it must not demand a second factor"
            );
        }
    }

    #[test]
    fn the_gate_runs_before_anything_is_signed() {

        for name in MOVES_MONEY {
            let body = body_of(name);
            let gate = body.find("guard_large_send(").expect("gated");
            for after in ["build_signed(", "broadcast(", "xmr_rpc::send("] {
                if let Some(at) = body.find(after) {
                    assert!(gate < at, "{name} reaches {after} before the gate");
                }
            }
        }
    }

    #[test]
    fn reveals_are_gated_too() {
        for name in ["reveal_seed", "reveal_monero_keys"] {
            let body = body_of(name);
            assert!(
                body.contains("require_2fa(") || body.contains("require_step_up("),
                "{name} shows a secret without passing the gate"
            );
        }
    }
}

#[cfg(test)]
mod address_style {
    use crate::chains::btc_tx::{Kind, Utxo};

    fn coin(value: u64, kind: Kind) -> Utxo {
        Utxo { txid: [0; 32], vout: 0, value, key_index: 0, kind }
    }

    #[test]
    fn prefers_money_then_the_newest_style() {
        assert_eq!(super::richest_kind(&[]), None);
        assert_eq!(super::richest_kind(&[coin(0, Kind::Legacy)]), None);
        assert_eq!(super::richest_kind(&[coin(274_599, Kind::Legacy), coin(10, Kind::Segwit)]), Some(Kind::Legacy.code()));
        assert_eq!(super::richest_kind(&[coin(5, Kind::Nested), coin(5, Kind::Segwit)]), Some(Kind::Segwit.code()));
        assert_eq!(super::richest_kind(&[coin(5, Kind::Nested), coin(5, Kind::Legacy)]), Some(Kind::Nested.code()));
    }
}
