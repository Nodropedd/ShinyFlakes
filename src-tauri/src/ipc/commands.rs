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
    /// The vault is sealed with a passphrase, so unlocking needs one on top
    /// of the seed phrase.
    pub needs_passphrase: bool,
    /// A vault file exists but its keychain key is gone, so nothing on this
    /// machine can decrypt it. The only way forward is to restore from the
    /// seed. Reported up front so the UI never shows a dead unlock form.
    pub key_missing: bool,
}

#[tauri::command]
pub fn vault_status(state: State<AppState>) -> VaultStatus {
    let initialized = store::exists(&state.vault_path);
    // A missing entry (NoVault) with a vault file present means the key is
    // gone. A transient keychain fault is not treated as "gone": the user
    // would hit it on unlock instead, rather than being pushed to restore.
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

/// New phrase for a first-run wallet. Nothing is written yet: the UI shows the
/// words for backup and only then calls create_vault.
#[tauri::command]
pub fn generate_mnemonic() -> Result<String> {
    Ok(seed::generate()?.to_string())
}

#[tauri::command]
pub fn create_vault(mnemonic: String, state: State<AppState>) -> Result<()> {
    let phrase = Zeroizing::new(mnemonic);

    if store::exists(&state.vault_path) {
        return Err(WalletError::VaultExists);
    }

    let parsed = seed::parse(&phrase)?;
    let key = keychain::load_or_create()?;

    let payload = VaultPayload {
        mnemonic: parsed.to_string(),
    };
    // A new wallet has no passphrase; one is added later from Settings.
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
    settle_dormancy_and_record_use(&state);
    resume_staying_signed_in(&state);

    Ok(())
}

/// What follows opening the wallet without the seed being typed just now.
fn settle_dormancy_and_record_use(state: &AppState) {
    // Whether this unlock follows a long silence has to be settled *before*
    // record_seen moves the marker it is measured against.
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

    // Using the wallet is what pushes the inactivity deadline back. A failed
    // attempt deliberately does not count.
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

    // Reject a malformed phrase before touching the keychain, so a typo does
    // not look like a storage fault.
    let parsed = seed::parse(&phrase)?;

    // The vault file exists, so a missing keychain entry is not "no wallet".
    // It means the wallet is here and undecryptable, which is a different
    // problem with a different fix.
    let key = keychain::load().map_err(|e| match e {
        WalletError::NoVault => WalletError::KeyMissing,
        other => other,
    })?;
    let pass = passphrase.as_deref().filter(|p| !p.is_empty());
    let payload = store::read(&state.vault_path, &key, pass)?;

    // The vault key comes from the OS keychain, so decryption succeeding does
    // not by itself prove the right seed was entered. Compare explicitly.
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

    // Using the wallet is what pushes the inactivity deadline back. A failed
    // attempt deliberately does not count.
    let _ = crate::inactivity::record_seen(&state.data_dir);
    resume_staying_signed_in(&state);

    Ok(())
}

#[tauri::command]
pub fn lock(state: State<AppState>) {
    pause_staying_signed_in(&state);
    state.wipe();
}

/// Full logout per the security model: every derived key is dropped and the
/// UI returns to seed entry.
///
/// This deliberately does NOT delete the encrypted vault or the keychain
/// entry. Wiping those on a mistyped phrase would be irreversible for anyone
/// without an offline backup, and the spec's "complete key wipe" is read here
/// as in-memory key material only. See README, open decisions.
#[tauri::command]
pub fn logout(state: State<AppState>) {
    pause_staying_signed_in(&state);
    state.wipe();
}

// ------------------------------------------------------------------------
// Staying signed in
//
// Unlocking asks for the seed phrase, not because the vault needs it — the
// keychain key opens it — but as proof that whoever is here holds the seed.
// Staying signed in drops that proof on start, at the owner's request: the
// app opens straight into the wallet. Anyone who can open the app on this
// device can then see and send everything, which is why it is off until
// asked for, held to the same two-factor gate as showing the seed, and
// paused by Lock until the phrase is typed again.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaySignedIn {
    /// The owner has turned it on.
    pub enabled: bool,
    /// It can work on this wallet. A vault passphrase has to be typed on every
    /// open, which rules it out.
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

/// Lock and logout pause it, so pressing Lock still means the next start asks
/// for the phrase. Best effort: a settings file that cannot be written leaves
/// the next start asking for the phrase anyway, since reading it fails too.
fn pause_staying_signed_in(state: &AppState) {
    if let Ok(mut cfg) = crate::appconfig::load(&state.data_dir) {
        if cfg.stay_signed_in && !cfg.sign_in_paused {
            cfg.sign_in_paused = true;
            let _ = crate::appconfig::save(&state.data_dir, &cfg);
        }
    }
}

/// Typing the phrase lifts a pause.
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

/// Turning it on is as good as leaving the seed on screen for whoever opens
/// the app next, so it is gated like revealing the seed. Turning it off never
/// is.
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

/// Opens the wallet on start without the seed phrase, when the owner asked
/// for that. Returns whether it did.
///
/// Anything in the way — the setting off, a Lock since the last unlock, a
/// vault passphrase, a missing key — is simply "no". The login screen then
/// asks for the phrase as it always has, and any real fault surfaces there
/// with its usual message rather than here.
#[tauri::command]
pub fn auto_unlock(state: State<AppState>) -> Result<bool> {
    if state.is_unlocked() {
        return Ok(true);
    }
    if !store::exists(&state.vault_path) {
        return Ok(false);
    }
    // The keychain first, and without creating anything: loading the settings
    // would make a fresh key if there were none, hiding a missing one.
    let Ok(key) = keychain::load() else {
        return Ok(false);
    };
    let Ok(cfg) = crate::appconfig::load(&state.data_dir) else {
        return Ok(false);
    };
    if !cfg.stay_signed_in || cfg.sign_in_paused {
        return Ok(false);
    }
    // No passphrase is given, so a vault that has one simply does not open.
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

    // Nobody typed the seed, so the long-silence check matters more here than
    // anywhere: with two-factor on, a wallet left unopened past the chosen
    // limit still asks for the second factor before it can be used.
    settle_dormancy_and_record_use(&state);
    Ok(true)
}

/// Receiving addresses for every asset, derived locally from the seed. Needs
/// no network, so this works before any node is configured.
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

/// Derives addresses, then asks each chain what they hold.
///
/// Every call reaches third-party endpoints and reveals these addresses and
/// this machine's IP to them. The UI says so before the first request.
#[tauri::command]
pub async fn fetch_balances(state: State<'_, AppState>) -> Result<Vec<AssetBalance>> {
    // The guard is dropped before any await: a std Mutex must not be held
    // across a suspension point.
    let addresses = {
        let guard = state
            .unlocked
            .lock()
            .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
        match guard.as_ref() {
            Some(session) => chains::addresses(session.seed.as_ref())?,
            None => return Err(WalletError::Locked),
        }
    };

    Ok(chains::rpc::balances(&addresses).await)
}

/// Spot prices in USD. Separate from balances so a price outage does not hide
/// the balances, and the other way round.
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
    /// The creator fee, an extra output to the donation address. Zero when the
    /// amount is tiny or the recipient is the fee address itself.
    pub creator_fee_minor: String,
    pub total_minor: String,
    /// True when a node executed the transfer in simulation and accepted it.
    pub simulated: bool,
}

/// The creator fee for a send, in the asset's smallest unit, using the
/// donation address for that asset.
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

/// Maps an asset onto the Bitcoin-style chain that handles it, if any.
fn btc_chain(asset: &str) -> Option<chains::btc_tx::Chain> {
    match asset {
        "BTC" => Some(chains::btc_tx::Chain::Bitcoin),
        "LTC" => Some(chains::btc_tx::Chain::Litecoin),
        _ => None,
    }
}

/// Everything needed to spend on a Bitcoin-style chain: our keys, the outputs
/// we hold, and what the network currently charges.
async fn btc_context(
    seed: &[u8; 64],
    chain: chains::btc_tx::Chain,
) -> Result<(Vec<chains::btc_tx::Keys>, Vec<chains::btc_tx::Utxo>, f64)> {
    let apis = chains::rpc::btc_apis(chain);

    let mut keyring: Vec<chains::btc_tx::Keys> = Vec::new();
    let mut utxos: Vec<chains::btc_tx::Utxo> = Vec::new();
    let mut next = 0u32;
    let mut empty_run = 0u32;

    // Gap-limit scan: keep walking forward until a stretch of addresses comes
    // back empty. A fresh wallet costs one batch of requests; a fragmented one
    // costs roughly as many as it actually uses, rather than a fixed window
    // that would cap how far a split could go.
    while next < chains::rpc::SCAN_CEILING && empty_run < chains::rpc::SCAN_GAP {
        let mut batch = Vec::new();
        for _ in 0..chains::rpc::SCAN_BATCH {
            if next >= chains::rpc::SCAN_CEILING {
                break;
            }
            let keys = chains::btc_tx::keys_at(seed, chain, next)?;
            batch.push((next, chains::btc_tx::own_address(&keys.pubkey_hash, chain)?));
            keyring.push(keys);
            next += 1;
        }

        for (_, mut found) in chains::rpc::esplora_utxos_batch(apis, &batch).await? {
            if found.is_empty() {
                empty_run += 1;
            } else {
                empty_run = 0;
                utxos.append(&mut found);
            }
        }
    }

    utxos.sort_by(|a, b| b.value.cmp(&a.value));
    let rate = chains::rpc::esplora_fee_rate(apis).await?;
    Ok((keyring, utxos, rate))
}

/// Ethereum amounts are wei, which needs the wider type.
fn parse_wei(amount: &str) -> Result<u128> {
    amount
        .trim()
        .parse::<u128>()
        .map_err(|_| WalletError::Derivation(format!("amount is not a whole number: {amount}")))
}

/// What an Ethereum send will cost and what it can afford.
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
    /// Largest amount that can actually be sent, which empties the account.
    pub max_minor: String,
    /// Any leftover must be zero or at least this, never in between.
    pub rent_minimum_minor: String,
}

/// What the account can actually afford, so the UI can offer a working
/// maximum instead of letting someone type their whole balance and be
/// refused by the network for the fee.
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
            // Bitcoin has no rent rule; the dust limit is what matters, and
            // that is enforced when the amount is checked.
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
            // Ethereum has no minimum balance rule.
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
        // A token transfer's fee is paid in the host chain's coin (SOL, ETH or
        // TRX), not in the token, so the whole token balance can leave.
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
        chains::sol_tx::signing_key(&seed)
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

/// Checks an amount against the account before anything is built. Returns the
/// fee so the caller can show it.
async fn check_affordable(seed: &[u8; 64], amount: u64) -> Result<u64> {
    let address = bs58::encode(
        chains::sol_tx::signing_key(seed).verifying_key().to_bytes(),
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

    // Solana purges an account that drops below the rent-exempt minimum, so
    // it refuses to leave one holding less than that. Emptying it entirely is
    // allowed; leaving dust is not.
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

// ---------- Tron (TRX and TRC-20) ----------

/// Verifies a node-built Tron transaction really carries our transfer, signs
/// its id locally, and broadcasts it. `expected` are lowercase-hex fragments
/// that must all appear in the raw data — the recipient, amount, and any
/// contract we chose — so a skeleton the node tampered with is caught before
/// it is ever signed.
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

/// Builds, verifies, signs and broadcasts a plain TRX transfer.
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

/// Builds, verifies, signs and broadcasts a TRC-20 transfer for a given
/// contract (USDT or USDC on Tron). The fee is paid in TRX (energy);
/// `FEE_LIMIT` only caps the burn, so only what the transfer uses is charged.
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

    // The raw data must carry both the transfer call (selector + our recipient
    // and amount) and the contract's own address.
    let data_hex = format!("a9059cbb{param_hex}");
    let contract_hex = chains::tron_tx::hex(&chains::tron_tx::parse_address(contract)?);
    tron_sign_and_send(seed, tx, &[data_hex, contract_hex]).await
}

// ---------- SPL and ERC-20 token sends ----------

fn sol_own_address(seed: &[u8; 64]) -> String {
    bs58::encode(chains::sol_tx::signing_key(seed).verifying_key().to_bytes()).into_string()
}

fn hexstr(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An SPL token balance for this wallet, in the token's smallest unit.
async fn spl_balance(seed: &[u8; 64], mint: &str) -> Result<u128> {
    chains::rpc::sol_spl_balance(&sol_own_address(seed), mint).await
}

/// Sends an SPL token (USDC or USDT on Solana).
///
/// Before signing, this reads our own token account for the mint from the node
/// and requires our locally derived associated-token-account to match it. That
/// both proves the derivation is correct for this run and makes the recipient's
/// derived account trustworthy — a wrong derivation aborts rather than sending
/// into an address the recipient could not reach.
async fn spl_send(seed: &[u8; 64], mint: &str, to: &str, amount: u64) -> Result<String> {
    let owner = sol_own_address(seed);
    let owner_key = chains::sol_tx::signing_key(seed).verifying_key().to_bytes();
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
    // The node runs it in simulation first, so a bad transfer fails here rather
    // than on chain.
    chains::rpc::sol_simulate(&tx).await?;
    chains::rpc::sol_broadcast(&tx).await
}

/// An ERC-20 token balance for this wallet, via `balanceOf`.
async fn erc20_balance(seed: &[u8; 64], contract: &str) -> Result<u128> {
    let keys = chains::eth::keys(seed)?;
    let data = format!("0x{}", hexstr(&chains::eth::erc20_balance_data(&keys.address)));
    chains::rpc::eth_view(contract, &data).await
}

/// Sends an ERC-20 token (USDC or USDT on Ethereum). Gas is paid in ETH; the
/// token balance is checked first so gas is not spent on a transfer that would
/// revert for insufficient funds.
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

    // ERC-20 transfers cost more than a plain send; this ceiling covers USDT's
    // heavier storage write with room to spare, and only gas actually used is
    // charged.
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

/// The network to send a token on: the caller's choice, or a sensible default
/// (USDC on Solana, USDT on Tron). Errors if the token does not live there.
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

/// The token balance this wallet holds for a token on a network.
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

/// Sends a token on the given network, routing to that chain's transfer.
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

/// Validates a recipient address against the network it will be sent on, so a
/// typo or a wrong-chain address is caught before anything is built.
fn validate_recipient(network: &str, to: &str) -> Result<()> {
    match network {
        "SOL" => chains::sol_tx::parse_address(to).map(|_| ()),
        "ETH" => chains::eth::parse_address(to).map(|_| ()),
        "TRON" => chains::tron_tx::parse_address(to).map(|_| ()),
        other => Err(WalletError::Unsupported(format!("unknown network {other}"))),
    }
}

/// Builds and signs the transfer, then has a node run it in simulation. This
/// broadcasts nothing, so it is safe to call while the user is still deciding.
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
        let (keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let change = chains::btc_tx::own_script(&keyring[0].pubkey_hash);
        // A fee output below the dust limit would be rejected, so it is
        // dropped rather than charged.
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
            // No node runs this in advance. Bitcoin has no simulation endpoint
            // the way Solana does, so the plan is checked locally only.
            simulated: false,
        });
    }

    if asset == "ETH" {
        let wei = parse_wei(&amount_minor)?;
        let (_keys, _address, balance, max_fee, _tip) = eth_context(&seed).await?;
        chains::eth::parse_address(&to)?;

        let per_tx_gas = max_fee * chains::eth::TRANSFER_GAS as u128;
        // The fee is a second transaction, so it costs gas of its own.
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
        // A creator fee rides a second transfer, so it needs its own headroom.
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
        // The fee is paid in the host chain's coin (SOL/ETH/TRX), and no
        // creator fee is added to a token send, so the token amount is exactly
        // what leaves.
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
    // The rent and balance checks must see everything leaving the account.
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

/// The Bitcoin-style fee output, or none when the fee is below the dust limit
/// and so would make the transaction invalid.
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

/// The destination output, plus the fee output when there is one.
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

/// Signs and broadcasts. Irreversible once the network accepts it, so the UI
/// must have shown a preview and taken an explicit confirmation first.
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
        // Rebuilt from scratch rather than reusing the preview: the set of
        // spendable outputs may have changed, and signing a stale plan risks
        // spending something already gone.
        let (keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let change = chains::btc_tx::own_script(&keyring[0].pubkey_hash);
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

        // Providers return the id; fall back to computing it ourselves if one
        // answers with something else.
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

        // Ethereum cannot pay two recipients in one transfer, so the fee rides
        // a second transaction at the next nonce. The main send has already
        // gone; a failure here loses only the fee, not the user's payment.
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
        // The creator fee rides a second transfer, as Ethereum's does. The
        // main send has already gone; a failure here costs only the fee.
        if creator > 0 {
            if let Some(fee_to) = donation_for("TRON") {
                let _ = tron_send_trx(&seed, &fee_to, creator).await;
            }
        }
        return Ok(id);
    }

    if chains::tokens::is_token(&asset) {
        let net = token_network(&asset, &network)?;
        // No creator fee on a token send: it would mean a second transfer and a
        // second host-chain fee, out of proportion to the fee itself.
        return token_send(&seed, &asset, &net, &to, amount as u128).await;
    }

    if asset != "SOL" {
        return Err(WalletError::Unsupported(format!(
            "Sending {asset} is not implemented yet."
        )));
    }

    let creator_fee = creator_fee as u64;
    check_affordable(&seed, amount + creator_fee).await?;

    // A fresh blockhash and one more simulation: the balance or the network
    // may have moved since the preview, and it costs nothing to check again.
    let blockhash = chains::rpc::sol_latest_blockhash().await?;
    let fee_to = donation_for("SOL").unwrap_or_default();
    let tx = chains::sol_tx::signed_transfer_with_fee(
        &seed, &to, amount, &fee_to, creator_fee, &blockhash,
    )?;
    chains::rpc::sol_simulate(&tx).await?;

    chains::rpc::sol_broadcast(&tx).await
}

/// Discards the wallet held on this machine and returns to a cold start.
///
/// This is destructive and cannot be undone: it deletes the encrypted vault
/// and the key that decrypts it. Anyone without their seed phrase written
/// down loses access permanently, so the UI demands a typed confirmation
/// before calling it.
#[tauri::command]
pub fn forget_wallet(state: State<AppState>) -> Result<()> {
    state.wipe();

    match std::fs::remove_file(&state.vault_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(WalletError::Storage(e.to_string())),
    }

    keychain::forget()
}

/// Recent transactions across every chain that can report them.
#[tauri::command]
pub async fn fetch_activity(state: State<'_, AppState>) -> Result<Vec<chains::history::Entry>> {
    let addresses = {
        let guard = state
            .unlocked
            .lock()
            .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
        match guard.as_ref() {
            Some(session) => chains::addresses(session.seed.as_ref())?,
            None => return Err(WalletError::Locked),
        }
    };

    Ok(chains::history::all(&addresses).await)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxoEntry {
    pub txid: String,
    pub vout: u32,
    pub value_minor: String,
    /// Which sub-wallet holds it. Zero is the main receiving address.
    pub key_index: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UtxoState {
    pub asset: String,
    pub total_minor: String,
    pub outputs: Vec<UtxoEntry>,
    /// Distinct sub-wallets currently holding something.
    pub sources: usize,
    pub fee_rate: f64,
    /// Largest number of pieces the balance could be split into without any
    /// falling below the dust limit.
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

/// The current shape of the wallet's outputs on a UTXO chain.
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
            // Back to display order for anyone pasting it into an explorer.
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
    /// What stays on the main address afterwards.
    pub change_minor: String,
    /// How many outputs are being consumed.
    pub inputs_used: usize,
    /// How many sub-wallets had to be combined to fund it.
    pub sources: usize,
}

/// Plans a split without signing anything.
async fn fragment_plan(
    seed: &[u8; 64],
    chain: chains::btc_tx::Chain,
    amount: u64,
    pieces: u32,
) -> Result<(Vec<chains::btc_tx::Keys>, chains::btc_tx::FragmentPlan)> {
    let (keyring, utxos, rate) = btc_context(seed, chain).await?;

    // Index 0 stays the main address and takes the change; the pieces go to
    // the ones after it, so each lands in its own sub-wallet.
    let piece_scripts: Vec<Vec<u8>> = keyring
        .iter()
        .skip(1)
        .map(|k| chains::btc_tx::own_script(&k.pubkey_hash))
        .collect();
    let change = chains::btc_tx::own_script(&keyring[0].pubkey_hash);

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

/// Signs and broadcasts the split. Irreversible.
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

    // Replanned rather than reusing the preview: outputs may have been spent
    // in between, and signing a stale plan would fail at broadcast anyway.
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

/// Plans sweeping every output back onto the main address.
#[tauri::command]
pub async fn consolidate_preview(asset: String, state: State<'_, AppState>) -> Result<SendQuote> {
    let chain = require_utxo_chain(&asset)?;
    let seed = seed_copy(&state)?;
    let (keyring, utxos, rate) = btc_context(&seed, chain).await?;

    let dest = chains::btc_tx::own_script(&keyring[0].pubkey_hash);
    let plan = chains::btc_tx::consolidate(&utxos, dest, rate)?;
    let value = plan.outputs[0].value;

    Ok(SendQuote {
        asset,
        to: chains::btc_tx::own_address(&keyring[0].pubkey_hash, chain)?,
        amount_minor: value.to_string(),
        fee_minor: plan.fee.to_string(),
        // Consolidating is a move to your own address, so no creator fee.
        creator_fee_minor: "0".to_string(),
        total_minor: (value as u128 + plan.fee as u128).to_string(),
        simulated: false,
    })
}

/// Signs and broadcasts the sweep. Irreversible.
#[tauri::command]
pub async fn consolidate_execute(asset: String, state: State<'_, AppState>) -> Result<String> {
    let chain = require_utxo_chain(&asset)?;
    let seed = seed_copy(&state)?;
    let (keyring, utxos, rate) = btc_context(&seed, chain).await?;

    // A sweep carries no amount of its own: it moves whatever is there, so
    // the sum of the outputs is what the threshold has to be measured on.
    let swept: u64 = utxos.iter().map(|u| u.value).sum();
    guard_large_send(&state, &asset, swept, None).await?;

    let dest = chains::btc_tx::own_script(&keyring[0].pubkey_hash);
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

/// Resolves "txid:vout" strings back to the outputs they name.
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
            // A coin that has been spent since the list was drawn must not be
            // silently swapped for another one.
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

/// Every coin available to spend, for choosing between them.
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
                address: chains::btc_tx::own_address(&owner.pubkey_hash, chain)?,
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
    /// False when the chain reuses one account rather than rotating.
    pub rotates: bool,
}

/// The address to hand out for the next payment.
///
/// On Bitcoin and Litecoin this walks forward to the first address that has
/// never appeared on chain. Reusing one address links every payment ever made
/// to it, which is the cheapest privacy mistake there is to avoid.
///
/// Account-based chains keep a single address by design: rotating there means
/// a separate account per payment, which changes how balances are read and
/// buys much less, so they are left alone.
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

/// The private keys for the Monero account.
///
/// Monero cannot be restored from the BIP-39 phrase, because mapping one to
/// Monero keys is this wallet's own convention rather than a standard. These
/// two keys are therefore the only way to reach the account from any other
/// Monero wallet, and they are spending authority: anyone holding the spend
/// key owns the funds. That makes this a sensitive reveal, so it is gated by
/// two-factor when it is on and always sends a notice afterwards.
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
        // The account was created by this wallet, so nothing before that
        // matters and a restore can skip most of the chain.
        restore_height_hint: "the block height when you first received Monero here".into(),
    };

    Ok(out)
}

/// Checks the local Monero wallet daemon is up and holding this account.
#[tauri::command]
pub async fn xmr_status(
    endpoint: String,
    state: State<'_, AppState>,
) -> Result<chains::xmr_rpc::WalletStatus> {
    let seed = seed_copy(&state)?;
    let expected = chains::xmr::address(&seed)?;
    chains::xmr_rpc::status(&endpoint, &expected).await
}

/// Balance according to the local Monero wallet.
#[tauri::command]
pub async fn xmr_balance(endpoint: String) -> Result<chains::xmr_rpc::Balance> {
    chains::xmr_rpc::balance(&endpoint).await
}

/// Prices a Monero transfer by building it and throwing it away.
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

/// Sends Monero. Irreversible.
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

/// Runs the inactivity check, clearing the wallet if the period has passed.
///
/// Called at startup, before unlocking. It needs no keys, so it still works
/// for someone who has lost the phrase and cannot get in.
#[tauri::command]
pub fn inactivity_check(state: State<AppState>) -> crate::inactivity::Status {
    match crate::inactivity::check(&state.data_dir, &state.vault_path) {
        crate::inactivity::Outcome::Idle(s)
        | crate::inactivity::Outcome::Wiped(s)
        | crate::inactivity::Outcome::SweepDue(s) => s,
    }
}

/// Changes how long the wallet may sit unopened before the switch fires.
#[tauri::command]
pub fn inactivity_set_months(
    months: u32,
    state: State<AppState>,
) -> Result<crate::inactivity::Status> {
    crate::inactivity::set_months(&state.data_dir, months)
}

/// Chooses what the switch does: delete the local wallet, or sweep the
/// balances to the donation addresses first.
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

    // Donate mode sweeps unattended, which needs the seed with no one present
    // to type a passphrase. So the two cannot both be on.
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

/// Adds, changes, or removes the vault passphrase.
///
/// `current` is the passphrase in force now, needed to read the vault before
/// re-sealing it. `next` is what to set; an empty or absent value removes it.
/// The re-encrypted vault is read back before the change is called done, so a
/// derivation slip cannot leave the wallet unopenable.
#[tauri::command]
pub fn set_vault_passphrase(
    current: Option<String>,
    next: Option<String>,
    state: State<AppState>,
) -> Result<()> {
    // Requiring an unlocked session means whoever changes the passphrase has
    // already proven they hold the seed.
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

    // Verifies the current passphrase by decrypting with it.
    let payload = store::read(&state.vault_path, &key, read_pass)?;

    let next = next.as_deref().filter(|p| !p.is_empty());
    store::write(&state.vault_path, &key, next, &payload)?;

    // Prove the new sealing opens before treating the change as done.
    store::read(&state.vault_path, &key, next)?;

    // A passphrase and donate mode are mutually exclusive, so turning one on
    // stands the other down. The same goes for staying signed in, which needs
    // the vault to open with nothing typed.
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

/// One chain's outcome during an inactivity sweep.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepResult {
    pub asset: String,
    /// The transaction id, when something was sent.
    pub txid: Option<String>,
    /// Why nothing was sent, when that is expected (no balance, or a chain
    /// this wallet cannot yet sign for).
    pub skipped: Option<String>,
    /// A real failure, usually the network. Blocks the local wipe so the
    /// next launch tries again.
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

/// Reads the seed straight from the vault, without an unlock.
///
/// This is the path the inactivity sweep needs, since by definition nobody is
/// there to type the phrase. It works because the vault key lives in the OS
/// credential store, so the app can decrypt at rest whenever the OS account
/// is available. It is used for nothing else.
fn recover_seed(state: &State<AppState>) -> Result<[u8; 64]> {
    let key = keychain::load()?;
    // No passphrase is supplied: this path runs unattended. A vault sealed
    // with one therefore cannot be read here, which is exactly why donate mode
    // is refused while a passphrase is set.
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

/// Sends every spendable balance to the donation addresses.
///
/// Reuses the same signing this wallet uses for an ordinary send, so nothing
/// about moving the money is new here; only the orchestration is. Chains that
/// cannot yet sign (Tron), that need a running daemon nobody has started
/// (Monero), or whose tokens this wallet does not send (USDC, USDT) are
/// skipped and left for the seed phrase to recover.
async fn sweep_all(seed: &[u8; 64]) -> Vec<SweepResult> {
    let mut out = Vec::new();

    // Bitcoin and Litecoin: consolidate every output to the donation script.
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

    // Ethereum: send balance minus the gas a plain transfer costs.
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

    // Solana: empty the account to the donation address.
    if let Some(to) = donation_for("SOL") {
        let from = bs58::encode(
            chains::sol_tx::signing_key(seed).verifying_key().to_bytes(),
        )
        .into_string();
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

    // Chains this wallet cannot sweep, named so the outcome is not silent.
    out.push(SweepResult::skip("TRON", "sending Tron is not implemented"));
    out.push(SweepResult::skip("USDT", "sending Tether is not implemented"));
    out.push(SweepResult::skip("USDC", "sending USD Coin is not implemented"));
    out.push(SweepResult::skip(
        "XMR",
        "needs the Monero daemon, which only runs while signed in",
    ));

    out
}

/// Runs a sweep that the inactivity switch has decided is due, then completes
/// the switch by deleting the local wallet if every attempted chain sent.
///
/// The condition is re-checked here in Rust. The front end asking is not
/// enough to move funds; the deadline and grace must genuinely have passed.
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

    // A skip is expected and fine; only a real failure holds back the wipe so
    // the next launch can retry the chains that did not go through.
    let all_ok = results.iter().all(|r| r.error.is_none());
    crate::inactivity::finish_sweep(&state.data_dir, &state.vault_path, all_ok);

    Ok(results)
}

/// Where a tip goes, per asset. Fixed and compiled in.
#[tauri::command]
pub fn donation_addresses() -> Vec<crate::donation::Donation> {
    crate::donation::addresses()
}

/// Whether Tor is installed, running, and carrying requests.
#[tauri::command]
pub fn tor_state(state: State<AppState>) -> crate::tor::TorState {
    crate::tor::state(&state.data_dir, state.tor_running())
}

/// Installs Tor if needed, starts it, waits for it to connect, then routes
/// every remote lookup through it. Slow the first time: it downloads the Tor
/// bundle and then builds a circuit.
#[tauri::command]
pub async fn tor_start(state: State<'_, AppState>) -> Result<crate::tor::TorState> {
    let data_dir = state.data_dir.clone();

    crate::tor::install(&data_dir).await?;

    // Reuse a Tor already answering rather than starting a second that could
    // not bind the port.
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

    // Routing is switched on only after a circuit exists, so requests are
    // never sent to a proxy that cannot yet carry them.
    crate::tor::wait_until_ready().await?;
    crate::tor::set_routing(true);

    Ok(crate::tor::state(&data_dir, state.tor_running()))
}

/// Stops routing through Tor and shuts the process down.
#[tauri::command]
pub fn tor_stop(state: State<AppState>) -> crate::tor::TorState {
    crate::tor::set_routing(false);
    state.stop_tor();
    crate::tor::state(&state.data_dir, false)
}

/// Whether Monero is installed, set up, and running.
///
/// Running is decided by asking the port, not by whether this session started
/// the process. A daemon left from a previous run is still a working daemon.
#[tauri::command]
pub async fn monero_setup_state(
    state: State<'_, AppState>,
) -> Result<chains::xmr_setup::SetupState> {
    let data_dir = state.data_dir.clone();
    let endpoint = format!("http://127.0.0.1:{}/json_rpc", chains::xmr_setup::RPC_PORT);
    let live = chains::xmr_rpc::ping(&endpoint).await.is_ok();
    Ok(chains::xmr_setup::state(&data_dir, live))
}

/// The whole Monero setup, in one go.
///
/// Fetches Monero's official wallet daemon if it is missing, checks it
/// against a hash compiled into this program, starts it against a node, and
/// creates the wallet from the keys this seed derives.
///
/// The spend key travels from memory into Monero over loopback. This program
/// never writes it to a file and never displays it.
#[tauri::command]
pub async fn monero_setup_run(
    daemon: Option<String>,
    state: State<'_, AppState>,
) -> Result<chains::xmr_setup::SetupState> {
    let data_dir = state.data_dir.clone();
    let daemon = daemon
        .filter(|d| !d.trim().is_empty())
        .unwrap_or_else(|| chains::xmr_setup::DEFAULT_DAEMON.to_string());

    // Derived before anything slow, so a locked wallet fails immediately
    // rather than after a long download.
    let seed = seed_copy(&state)?;
    let keys = chains::xmr::keys(&seed)?;
    let address = chains::xmr::address(&seed)?;
    let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let spend = hex(keys.spend.to_bytes());
    let view = hex(keys.view.to_bytes());

    chains::xmr_setup::install(&data_dir).await?;

    // Anything this session started is stopped first; orphans from earlier
    // runs are dealt with inside ensure_running.
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

    // Reaching here means the daemon answered and the wallet opened, whether
    // it was started just now or adopted from a previous run.
    Ok(chains::xmr_setup::state(&data_dir, true))
}

/// Stops the Monero daemon, including one left over from an earlier run.
///
/// The wallet is asked to save and close first. That cannot protect the money,
/// which lives on the chain, but it protects the scan cache from being left
/// half-written and needing a slow rebuild.
#[tauri::command]
pub async fn monero_stop(
    state: State<'_, AppState>,
) -> Result<chains::xmr_setup::SetupState> {
    let data_dir = state.data_dir.clone();
    let endpoint = format!("http://127.0.0.1:{}/json_rpc", chains::xmr_setup::RPC_PORT);

    // Best effort: a daemon that has already gone will not answer, and that
    // is not a reason to refuse to clean up after it.
    let _ = chains::xmr_rpc::close_wallet(&endpoint).await;

    state.stop_monero();
    chains::xmr_setup::stop_any(&data_dir);
    Ok(chains::xmr_setup::state(&data_dir, false))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TwoFactorState {
    pub enabled: bool,
    /// A recent code check still authorises a reveal.
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
    /// The base32 secret, for manual entry into an authenticator.
    pub secret: String,
    /// The otpauth URI to render as a QR code.
    pub uri: String,
}

/// Begins turning two-factor on: mints a TOTP secret and hands back the QR to
/// scan. Nothing is saved until a code confirms the authenticator holds it, so
/// an abandoned setup leaves two-factor off.
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

/// Confirms setup: the code must match the pending secret, which proves the
/// authenticator imported it. Only then is two-factor saved as on.
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
        // A fresh, valid code doubles as a pass, so the setup can flow straight
        // into whatever prompted it.
        tf.pass_expiry = Some(crate::twofa::pass_expires_at());
        tf.wrong = 0;
        tf.locked_until = None;
    }
    Ok(true)
}

/// Turns two-factor off and forgets the secret.
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
    /// ok, wrong, lockedOut, or none.
    pub status: String,
    /// Tries left before the lockout, when wrong.
    pub remaining: Option<u32>,
    /// Unix second the lockout lifts, when locked out.
    pub locked_until: Option<i64>,
}

/// Checks a code from the authenticator and, if right, grants a short-lived
/// pass a reveal can consume. A run of wrong codes locks the check briefly so
/// the six-digit space cannot be walked through while one is valid.
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

/// The seed-phrase fallback for two-factor, for when the authenticator is not
/// to hand. Proving the seed is at least as strong as a code, since the seed is
/// the wallet, so a correct phrase grants the same short-lived pass.
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

/// Requires a valid two-factor pass when two-factor is on, and consumes it.
///
/// The gate in front of anything worth stopping for.
///
/// With no factor set up this is a no-op: the session is already unlocked,
/// which needed the seed, and demanding a factor nobody can produce would be a
/// lockout rather than a protection. Otherwise every factor the policy asks
/// for must have been cleared recently, and each one is consumed here so the
/// next guarded action needs its own.
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

    // An unanswered return-after-silence blocks everything guarded, not just
    // the moment of unlocking.
    if tf.dormant_pending || (need.totp && !totp_ok) {
        return Err(WalletError::TwoFactorRequired);
    }
    if need.totp {
        tf.pass_expiry = None;
    }
    Ok(())
}

/// Revealing a secret, which is the oldest of the guarded actions.
fn require_2fa(state: &State<AppState>) -> Result<()> {
    require_step_up(state, crate::twofa::Guarded::RevealSecret)
}

/// The seed phrase, for backup. Gated by two-factor when it is on.
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

// ------------------------------------------------------------------------
// Swaps (ChangeNOW, non-custodial)
//
// The wallet gets a rate and a deposit address from ChangeNOW, then pays a
// normal on-chain send to it from the "from" coin. The proceeds arrive at an
// address this wallet owns. No custody, no account of ours; keys never leave
// the machine, and the calls ride Tor when it is on. The funding send does NOT
// carry the creator fee: it would change the exact deposit the exchange waits
// for.
//
// The ChangeNOW API key belongs to whoever builds and hands out the program,
// not the end user, so it is compiled in (see swapcfg.rs) rather than
// configured per install; the partner commission is attributed to that key and
// set in ChangeNOW's dashboard. Swaps only work once a key has been baked in.
// ------------------------------------------------------------------------

/// The compiled-in ChangeNOW key, or a clear error when no key was built in.
/// Returned wrapped so it is wiped from memory once the swap call is done.
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

/// This wallet's own address for an asset, for a swap's payout or refund.
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
    /// Expected proceeds. A variable-rate swap can settle a little different.
    pub amount_to_minor: String,
    pub provider: String,
}

/// A rate for a swap. Reaches ChangeNOW; broadcasts nothing.
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
    /// Where the "from" coin must be paid.
    pub deposit_address: String,
    /// A tag some chains need. Empty for the ones this wallet sends.
    pub deposit_memo: String,
    /// Exactly how much to pay, in the "from" asset's smallest unit.
    pub deposit_amount_minor: String,
    /// Where the proceeds will land: an address this wallet owns.
    pub payout_address: String,
    pub amount_to_minor: String,
    pub provider: String,
    pub status: String,
}

/// Creates a trade: a deposit address and a locked-in amount. Still broadcasts
/// nothing; the funding send is a separate, confirmed step.
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

    // The deposit ChangeNOW expects, back in our units. It should match what we
    // asked for; if the provider echoes a rounded figure, that is what is owed.
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

/// Pays the deposit for a created trade. A normal, locally-signed send with no
/// creator fee. Irreversible once broadcast, so the UI must confirm first.
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

    // None of this wallet's from-chains attach a deposit tag to a plain
    // transfer. If a route needs one, paying it from here would send to the
    // right address without the tag and the funds could be lost, so refuse.
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
        let change = chains::btc_tx::own_script(&keyring[0].pubkey_hash);
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

    // Swaps deposit USDC on Solana and USDT on Tron (see chains::swap::pair),
    // so fund each on the network the trade expects.
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

/// The current status of a trade, for polling until the proceeds arrive.
#[tauri::command]
pub async fn swap_status(id: String) -> Result<String> {
    let key = swap_creds()?;
    chains::swap::status(&key, &id).await
}


/// What one transfer is worth in USD.
///
/// Priced from the same quotes the portfolio total uses, so the two sides of
/// the threshold cannot disagree. `None` when the asset has no known decimals
/// or no quote, which the caller must treat as unpriced rather than as zero.
fn transfer_usd(
    asset: &str,
    amount_minor: u64,
    prices: &HashMap<String, chains::rpc::Quote>,
) -> Option<f64> {
    let dp = chains::swap::decimals(asset)?;
    let quote = prices.get(asset)?;
    Some(amount_minor as f64 / 10f64.powi(dp as i32) * quote.price)
}

/// Stops a transfer large enough to be worth asking for a second factor.
///
/// Every path that can move money calls this before it signs anything, and it
/// is one shared function rather than a check copied per command on purpose:
/// the first version of this lived in `send_preview` by mistake, so previewing
/// a send asked for a code while broadcasting one did not.
///
/// The value is worked out here from this side's own price feed. The figure
/// the interface passes for the creator fee is only a fallback, because a
/// number supplied by the caller is one that anyone who already owns the
/// caller could understate to slip under the threshold.
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
    // With no factor set up there is nothing to ask for, so skip the network
    // work rather than pricing a wallet for a gate that cannot fire.
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

/// The portfolio total against an already-fetched set of quotes, so a caller
/// that has just priced something does not pay for a second round trip.
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
        // Stablecoins arrive once per network and again as a total. Only the
        // totals carry no network, so this counts each holding exactly once.
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

// ---------------- Step-up policy ----------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepUpView {
    pub dormant_days: u32,
    pub large_send: bool,
    /// Whether the authenticator is actually set up, so the UI can say why
    /// nothing is being asked for.
    pub totp_available: bool,
    /// The thresholds, so the interface can state them rather than hardcode.
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
    // A year of silence is the longest that still means anything; past that
    // the inactivity switch owns the machine anyway.
    cfg.step_up.dormant_days = dormant_days.min(365);
    cfg.step_up.large_send = large_send;
    crate::appconfig::save(&state.data_dir, &cfg)?;
    step_up_view(&state.data_dir)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub totp: bool,
    /// Already satisfied in this session, so the prompt can skip ahead.
    pub totp_done: bool,
}

fn guarded_from(action: &str) -> crate::twofa::Guarded {
    match action {
        "send" => crate::twofa::Guarded::LargeSend,
        "dormant" => crate::twofa::Guarded::Dormant,
        _ => crate::twofa::Guarded::RevealSecret,
    }
}

/// What the user still has to produce for `action`.
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

/// Whether this session still owes a factor for returning after a long
/// silence. The interface asks right after unlocking.
#[tauri::command]
pub fn dormant_step_up_pending(state: State<AppState>) -> Result<bool> {
    let tf = state
        .two_factor
        .lock()
        .map_err(|_| WalletError::Storage("2fa lock poisoned".into()))?;
    Ok(tf.dormant_pending)
}

/// Clears that debt once the required factors have been produced.
///
/// Checks the passes directly rather than going through the gate, which would
/// refuse while the flag it is trying to clear is still set.
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

    // Consumed here, so clearing the dormancy debt does not also hand over a
    // free pass to the next reveal.
    if need.totp {
        tf.pass_expiry = None;
    }
    tf.dormant_pending = false;
    Ok(true)
}

/// Whether the settings file is present but unreadable, so the interface can
/// offer the reset rather than throwing a decryption error at every panel.
#[tauri::command]
pub fn settings_unreadable(state: State<AppState>) -> Result<bool> {
    Ok(crate::appconfig::is_unreadable(&state.data_dir))
}

/// Puts the unreadable settings aside and starts fresh. Explicit, never
/// automatic: see appconfig::reset for why.
#[tauri::command]
pub fn reset_settings(state: State<AppState>) -> Result<()> {
    if !state.is_unlocked() {
        return Err(WalletError::Locked);
    }
    crate::appconfig::reset(&state.data_dir)
}

#[cfg(test)]
mod gate_wiring {
    //! The large-send gate is policy plus wiring, and only the policy has
    //! unit tests that can see it. The first version of the wiring sat in
    //! `send_preview` instead of `send_execute`, so previewing a transfer
    //! asked for a code and broadcasting one did not — a hole no test of
    //! `is_large_send` could ever have caught.
    //!
    //! These read the source of this file. That is unusual, and it is the
    //! point: the bug was *which function* the call lives in, which is a
    //! property of the text rather than of any value at run time.

    const SOURCE: &str = include_str!("commands.rs");

    /// The body of a top-level `fn`, from its signature to the next one.
    fn body_of(name: &str) -> &'static str {
        let sig = format!("\npub async fn {name}(");
        let start = SOURCE
            .find(&sig)
            .unwrap_or_else(|| panic!("{name} is not in this file any more"));
        let rest = &SOURCE[start + sig.len()..];
        let end = rest.find("\npub ").unwrap_or(rest.len());
        &rest[..end]
    }

    /// Every command that signs or broadcasts a transfer.
    const MOVES_MONEY: [&str; 5] = [
        "send_execute",
        "fragment_execute",
        "consolidate_execute",
        "xmr_send",
        "swap_fund",
    ];

    /// Commands that only quote or plan. Gating these is not a security
    /// problem but it is a bug: it asks for a code to look at a number, and
    /// it drags a full portfolio valuation onto every keystroke.
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
        // Ordering is the whole value of the check: a gate after the
        // signature has already been broadcast protects nothing.
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
