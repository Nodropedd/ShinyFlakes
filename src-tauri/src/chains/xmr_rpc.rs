//! Monero wallet-RPC calls.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Result, WalletError};

pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:18082/json_rpc";

const TIMEOUT: Duration = Duration::from_secs(60);

fn client() -> Result<reqwest::Client> {
    crate::http_client::builder()
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

#[derive(Deserialize)]
struct VersionResult {
    #[allow(dead_code)]
    version: u32,
}

pub async fn ping(endpoint: &str) -> Result<()> {
    let _: VersionResult = call(endpoint, "get_version", serde_json::json!({})).await?;
    Ok(())
}

#[derive(Deserialize)]
struct Empty {}

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

pub async fn close_wallet(endpoint: &str) -> Result<()> {
    let _: Empty = call(endpoint, "store", serde_json::json!({})).await?;
    let _: Empty = call(endpoint, "close_wallet", serde_json::json!({})).await?;
    Ok(())
}

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

            let text = e.to_string();
            if text.contains("already open") {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

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

    pub matches_wallet: bool,
}

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

#[derive(Deserialize)]
struct BalanceResult {
    balance: u64,
    unlocked_balance: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Balance {

    pub total_minor: String,

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

    #[serde(default)]
    pub creator_fee_minor: String,
}

pub async fn estimate(
    endpoint: &str,
    to: &str,
    amount: u64,
    fee_to: Option<&str>,
    fee_amount: u64,
) -> Result<Transfer> {

    transfer(endpoint, to, amount, fee_to, fee_amount, true).await
}

async fn transfer(
    endpoint: &str,
    to: &str,
    amount: u64,
    fee_to: Option<&str>,
    fee_amount: u64,
    do_not_relay: bool,
) -> Result<Transfer> {
    let mut destinations = vec![serde_json::json!({ "amount": amount, "address": to })];
    if let (Some(fee_to), true) = (fee_to, fee_amount > 0) {
        if fee_to != to {
            destinations.push(serde_json::json!({ "amount": fee_amount, "address": fee_to }));
        }
    }

    let result: TransferResult = call(
        endpoint,
        "transfer",
        serde_json::json!({
            "destinations": destinations,
            "account_index": 0,
            "priority": 0,
            "do_not_relay": do_not_relay,
            "get_tx_key": false,
        }),
    )
    .await?;

    Ok(Transfer {
        tx_hash: result.tx_hash,
        fee_minor: result.fee.to_string(),
        amount_minor: result.amount.to_string(),

        creator_fee_minor: fee_amount.to_string(),
    })
}

pub async fn send(
    endpoint: &str,
    to: &str,
    amount: u64,
    fee_to: Option<&str>,
    fee_amount: u64,
) -> Result<Transfer> {
    transfer(endpoint, to, amount, fee_to, fee_amount, false).await
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
