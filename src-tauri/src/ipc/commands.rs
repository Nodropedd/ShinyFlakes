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
use crate::store::{self, Bucket, VaultPayload};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub initialized: bool,
    pub unlocked: bool,
}

#[tauri::command]
pub fn vault_status(state: State<AppState>) -> VaultStatus {
    VaultStatus {
        initialized: store::exists(&state.vault_path),
        unlocked: state.is_unlocked(),
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
        buckets: Vec::new(),
    };
    store::write(&state.vault_path, &key, &payload)?;

    let mut guard = state
        .unlocked
        .lock()
        .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;
    *guard = Some(Unlocked {
        seed: seed::to_seed(&parsed),
        mnemonic: Zeroizing::new(parsed.to_string()),
        buckets: payload.buckets.clone(),
    });

    Ok(())
}

#[tauri::command]
pub fn unlock(mnemonic: String, state: State<AppState>) -> Result<()> {
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
    let payload = store::read(&state.vault_path, &key)?;

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
        buckets: payload.buckets.clone(),
    });

    Ok(())
}

#[tauri::command]
pub fn lock(state: State<AppState>) {
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
    state.wipe();
}

#[tauri::command]
pub fn list_buckets(state: State<AppState>) -> Result<Vec<Bucket>> {
    let guard = state
        .unlocked
        .lock()
        .map_err(|_| WalletError::Storage("session lock poisoned".into()))?;

    match guard.as_ref() {
        Some(session) => Ok(session.buckets.clone()),
        None => Err(WalletError::Locked),
    }
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
    pub total_minor: String,
    /// True when a node executed the transfer in simulation and accepted it.
    pub simulated: bool,
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
pub async fn send_limits(asset: String, state: State<'_, AppState>) -> Result<SendLimits> {
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

/// Builds and signs the transfer, then has a node run it in simulation. This
/// broadcasts nothing, so it is safe to call while the user is still deciding.
#[tauri::command]
pub async fn send_preview(
    asset: String,
    to: String,
    amount_minor: String,
    outpoints: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<SendQuote> {
    let amount = parse_minor(&amount_minor)?;
    let seed = seed_copy(&state)?;

    if let Some(chain) = btc_chain(&asset) {
        let (keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let dest = chains::btc_tx::script_pubkey_for(&to, chain)?;
        let change = chains::btc_tx::own_script(&keyring[0].pubkey_hash);

        let plan = match &outpoints {
            Some(chosen) => {
                let picked = pick_outputs(&utxos, chosen)?;
                chains::btc_tx::plan_with(&picked, dest, amount, change, rate)?
            }
            None => chains::btc_tx::select(&utxos, dest, amount, change, rate)?,
        };

        return Ok(SendQuote {
            asset,
            to,
            amount_minor: amount.to_string(),
            fee_minor: plan.fee.to_string(),
            total_minor: (amount as u128 + plan.fee as u128).to_string(),
            // No node runs this in advance. Bitcoin has no simulation endpoint
            // the way Solana does, so the plan is checked locally only.
            simulated: false,
        });
    }

    if asset == "ETH" {
        let wei = parse_wei(&amount_minor)?;
        let (_keys, _address, balance, max_fee, _tip) = eth_context(&seed).await?;
        let to_address = chains::eth::parse_address(&to)?;
        let _ = to_address;

        let cost = max_fee * chains::eth::TRANSFER_GAS as u128;
        if wei + cost > balance {
            return Err(WalletError::Funds(format!(
                "This needs {} wei including up to {cost} of gas, but the account holds {balance}.",
                wei + cost
            )));
        }

        return Ok(SendQuote {
            asset,
            to,
            amount_minor: wei.to_string(),
            fee_minor: cost.to_string(),
            total_minor: (wei + cost).to_string(),
            // The fee shown is the ceiling. Anything above the base fee at
            // inclusion time is refunded, so the real cost is usually lower.
            simulated: false,
        });
    }

    if asset != "SOL" {
        return Err(WalletError::Unsupported(format!(
            "Sending {asset} is not implemented yet."
        )));
    }

    let fee = check_affordable(&seed, amount).await?;
    let blockhash = chains::rpc::sol_latest_blockhash().await?;
    let tx = chains::sol_tx::signed_transfer(&seed, &to, amount, &blockhash)?;
    chains::rpc::sol_simulate(&tx).await?;

    Ok(SendQuote {
        asset,
        to,
        amount_minor: amount.to_string(),
        fee_minor: fee.to_string(),
        total_minor: (amount as u128 + fee as u128).to_string(),
        simulated: true,
    })
}

/// Signs and broadcasts. Irreversible once the network accepts it, so the UI
/// must have shown a preview and taken an explicit confirmation first.
#[tauri::command]
pub async fn send_execute(
    asset: String,
    to: String,
    amount_minor: String,
    outpoints: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<String> {
    let amount = parse_minor(&amount_minor)?;
    let seed = seed_copy(&state)?;

    if let Some(chain) = btc_chain(&asset) {
        // Rebuilt from scratch rather than reusing the preview: the set of
        // spendable outputs may have changed, and signing a stale plan risks
        // spending something already gone.
        let (keyring, utxos, rate) = btc_context(&seed, chain).await?;
        let dest = chains::btc_tx::script_pubkey_for(&to, chain)?;
        let change = chains::btc_tx::own_script(&keyring[0].pubkey_hash);

        let plan = match &outpoints {
            Some(chosen) => {
                let picked = pick_outputs(&utxos, chosen)?;
                chains::btc_tx::plan_with(&picked, dest, amount, change, rate)?
            }
            None => chains::btc_tx::select(&utxos, dest, amount, change, rate)?,
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

        let cost = max_fee * chains::eth::TRANSFER_GAS as u128;
        if wei + cost > balance {
            return Err(WalletError::Funds(format!(
                "This needs {} wei including gas, but the account holds {balance}.",
                wei + cost
            )));
        }

        let transfer = chains::eth::Transfer {
            nonce: chains::rpc::eth_nonce(&address).await?,
            max_priority_fee: tip,
            max_fee,
            gas_limit: chains::eth::TRANSFER_GAS,
            to: chains::eth::parse_address(&to)?,
            value: wei,
            data: Vec::new(),
        };

        let signed = chains::eth::sign(&keys, &transfer)?;
        return chains::rpc::eth_broadcast(&signed).await;
    }

    if asset != "SOL" {
        return Err(WalletError::Unsupported(format!(
            "Sending {asset} is not implemented yet."
        )));
    }

    check_affordable(&seed, amount).await?;

    // A fresh blockhash and one more simulation: the balance or the network
    // may have moved since the preview, and it costs nothing to check again.
    let blockhash = chains::rpc::sol_latest_blockhash().await?;
    let tx = chains::sol_tx::signed_transfer(&seed, &to, amount, &blockhash)?;
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
/// key owns the funds.
#[tauri::command]
pub fn reveal_monero_keys(state: State<AppState>) -> Result<MoneroKeys> {
    let seed = seed_copy(&state)?;
    let keys = chains::xmr::keys(&seed)?;

    let hex = |bytes: [u8; 32]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();

    Ok(MoneroKeys {
        address: chains::xmr::address(&seed)?,
        spend_key: hex(keys.spend.to_bytes()),
        view_key: hex(keys.view.to_bytes()),
        // The account was created by this wallet, so nothing before that
        // matters and a restore can skip most of the chain.
        restore_height_hint: "the block height when you first received Monero here".into(),
    })
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
) -> Result<chains::xmr_rpc::Transfer> {
    let amount = parse_minor(&amount_minor)?;
    chains::xmr_rpc::estimate(&endpoint, &to, amount).await
}

/// Sends Monero. Irreversible.
#[tauri::command]
pub async fn xmr_send(
    endpoint: String,
    to: String,
    amount_minor: String,
) -> Result<chains::xmr_rpc::Transfer> {
    let amount = parse_minor(&amount_minor)?;
    chains::xmr_rpc::send(&endpoint, &to, amount).await
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
#[tauri::command]
pub fn monero_stop(state: State<AppState>) -> chains::xmr_setup::SetupState {
    state.stop_monero();
    chains::xmr_setup::stop_any(&state.data_dir);
    chains::xmr_setup::state(&state.data_dir, false)
}
