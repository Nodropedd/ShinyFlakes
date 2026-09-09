//! Bridge to a Monero wallet running on this machine.
//!
//! Monero balances cannot be read from an address, and spending needs ring
//! signatures and range proofs. Rather than reimplement that, this talks to
//! `monero-wallet-rpc`, the official wallet daemon, over localhost.
//!
//! That keeps the promise the rest of the wallet makes: no third party is
//! involved, because the process being asked is one the user runs themselves.
//! The cryptography stays with the implementation the Monero project
//! maintains, which is where it belongs.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Result, WalletError};

/// Where monero-wallet-rpc listens by default.
pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:18082/json_rpc";

/// Scanning can take a while on a wallet that has just been restored, so this
/// is far more patient than the other chains.
const TIMEOUT: Duration = Duration::from_secs(60);

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| WalletError::Network(e.to_string()))
}

fn net(e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(format!("monero wallet: {e}"))
}

#[derive(Deserialize)]
struct RpcError {
    message: String,
}

#[derive(Deserialize)]
struct RpcEnvelope<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

/// Rejects anything that is not a local address.
///
/// A remote wallet daemon would hold the spend key, so pointing this at one
/// would hand someone else the money. Only loopback is accepted.
fn check_local(endpoint: &str) -> Result<()> {
    let host = endpoint
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .and_then(|hostport| hostport.rsplit_once(':').map(|(h, _)| h).or(Some(hostport)))
        .unwrap_or("");

    let local = matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1");
    if local {
        Ok(())
    } else {
        Err(WalletError::Unsupported(format!(
            "The Monero wallet must run on this machine. {host} is not local, and a remote \
             wallet daemon would hold your spend key."
        )))
    }
}

async fn call<T: serde::de::DeserializeOwned>(
    endpoint: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<T> {
    // An empty setting means "wherever it normally lives", so the caller does
    // not have to know the port.
    let endpoint = if endpoint.trim().is_empty() {
        DEFAULT_ENDPOINT
    } else {
        endpoint.trim()
    };
    check_local(endpoint)?;

    let body: RpcEnvelope<T> = client()?
        .post(endpoint)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": "0", "method": method, "params": params
        }))
        .send()
        .await
        .map_err(|e| {
            // The usual failure is simply that nothing is listening, and the
            // raw connection error does not say what to do about it.
            WalletError::Network(format!(
                "Could not reach monero-wallet-rpc at {endpoint}. Is it running? ({e})"
            ))
        })?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;

    match (body.result, body.error) {
        (Some(result), _) => Ok(result),
        (None, Some(e)) => Err(WalletError::Network(format!("monero wallet: {}", e.message))),
        (None, None) => Err(net("empty response")),
    }
}

// ---------- lifecycle ----------

#[derive(Deserialize)]
struct VersionResult {
    #[allow(dead_code)]
    version: u32,
}

/// Cheapest call that proves the daemon is listening. Works with no wallet
/// open, which is how it is used while waiting for startup.
pub async fn ping(endpoint: &str) -> Result<()> {
    let _: VersionResult = call(endpoint, "get_version", serde_json::json!({})).await?;
    Ok(())
}

#[derive(Deserialize)]
struct Empty {}

/// Creates a wallet file from an address and its two private keys.
///
/// The keys go from this process into Monero over loopback and are not
/// written anywhere by this program.
#[allow(clippy::too_many_arguments)]
pub async fn generate_from_keys(
    endpoint: &str,
    filename: &str,
    address: &str,
    spend_key: &str,
    view_key: &str,
    password: &str,
    restore_height: u64,
) -> Result<()> {
    let _: Empty = call(
        endpoint,
        "generate_from_keys",
        serde_json::json!({
            "filename": filename,
            "address": address,
            "spendkey": spend_key,
            "viewkey": view_key,
            "password": password,
            "restore_height": restore_height,
            "autosave_current": true,
        }),
    )
    .await?;
    Ok(())
}

/// Writes the wallet cache to disk and closes it.
///
/// Killing the daemon outright cannot lose money, since that lives on the
/// chain and the keys come from the seed, but it can leave the scan cache
/// half-written and force a slow rescan. Asking first avoids that.
pub async fn close_wallet(endpoint: &str) -> Result<()> {
    let _: Empty = call(endpoint, "store", serde_json::json!({})).await?;
    let _: Empty = call(endpoint, "close_wallet", serde_json::json!({})).await?;
    Ok(())
}

/// Opens an existing wallet file. Harmless if it is already open.
pub async fn open_wallet(endpoint: &str, filename: &str, password: &str) -> Result<()> {
    match call::<Empty>(
        endpoint,
        "open_wallet",
        serde_json::json!({ "filename": filename, "password": password }),
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(e) => {
            // Asking to open the wallet that is already open is not a
            // failure worth surfacing.
            let text = e.to_string();
            if text.contains("already open") {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

// ---------- status ----------

#[derive(Deserialize)]
struct AddressResult {
    address: String,
}

#[derive(Deserialize)]
struct HeightResult {
    height: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletStatus {
    pub address: String,
    pub height: u64,
    /// True when the daemon is serving the account this wallet derives.
    pub matches_wallet: bool,
}

/// Checks the bridge is up and serving the right account.
///
/// A wallet file restored from someone else's keys would happily answer, so
/// the address is compared rather than assumed.
pub async fn status(endpoint: &str, expected_address: &str) -> Result<WalletStatus> {
    let address: AddressResult =
        call(endpoint, "get_address", serde_json::json!({ "account_index": 0 })).await?;
    let height: HeightResult = call(endpoint, "get_height", serde_json::json!({})).await?;

    Ok(WalletStatus {
        matches_wallet: address.address == expected_address,
        address: address.address,
        height: height.height,
    })
}

// ---------- balance ----------

#[derive(Deserialize)]
struct BalanceResult {
    balance: u64,
    unlocked_balance: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {
    /// Everything the wallet can see, in atomic units.
    pub total_minor: String,
    /// The part that is spendable now. Change needs ten blocks to unlock.
    pub unlocked_minor: String,
}

pub async fn balance(endpoint: &str) -> Result<Balance> {
    let result: BalanceResult =
        call(endpoint, "get_balance", serde_json::json!({ "account_index": 0 })).await?;

    Ok(Balance {
        total_minor: result.balance.to_string(),
        unlocked_minor: result.unlocked_balance.to_string(),
    })
}

// ---------- sending ----------

#[derive(Deserialize)]
struct TransferResult {
    tx_hash: String,
    fee: u64,
    amount: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub tx_hash: String,
    pub fee_minor: String,
    pub amount_minor: String,
}

/// Works out what a transfer would cost without sending it.
///
/// Monero fees depend on the size of the transaction, which depends on how
/// many outputs have to be gathered, so it cannot be predicted from the
/// amount alone. The wallet builds the real thing and reports the fee.
pub async fn estimate(endpoint: &str, to: &str, amount: u64) -> Result<Transfer> {
    let result: TransferResult = call(
        endpoint,
        "transfer",
        serde_json::json!({
            "destinations": [{ "amount": amount, "address": to }],
            "account_index": 0,
            "priority": 0,
            // Built and priced, then thrown away rather than broadcast.
            "do_not_relay": true,
            "get_tx_key": false,
        }),
    )
    .await?;

    Ok(Transfer {
        tx_hash: result.tx_hash,
        fee_minor: result.fee.to_string(),
        amount_minor: result.amount.to_string(),
    })
}

/// Builds, signs and broadcasts. Irreversible.
pub async fn send(endpoint: &str, to: &str, amount: u64) -> Result<Transfer> {
    let result: TransferResult = call(
        endpoint,
        "transfer",
        serde_json::json!({
            "destinations": [{ "amount": amount, "address": to }],
            "account_index": 0,
            "priority": 0,
            "get_tx_key": false,
        }),
    )
    .await?;

    Ok(Transfer {
        tx_hash: result.tx_hash,
        fee_minor: result.fee.to_string(),
        amount_minor: result.amount.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_loopback_only() {
        assert!(check_local("http://127.0.0.1:18082/json_rpc").is_ok());
        assert!(check_local("http://localhost:18082/json_rpc").is_ok());
        assert!(check_local("http://[::1]:18082/json_rpc").is_ok());
    }

    #[test]
    fn refuses_a_remote_wallet_daemon() {
        // A remote daemon holds the spend key, so this must never be allowed
        // through, however it is dressed up.
        assert!(check_local("http://192.168.1.50:18082/json_rpc").is_err());
        assert!(check_local("https://monero.example.com/json_rpc").is_err());
        assert!(check_local("http://127.0.0.1.evil.com:18082/json_rpc").is_err());
        assert!(check_local("http://not-localhost:18082/json_rpc").is_err());
    }

    #[test]
    fn the_default_endpoint_is_local() {
        assert!(check_local(DEFAULT_ENDPOINT).is_ok());
    }
}
