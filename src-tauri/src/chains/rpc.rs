//! Node calls, Tor-aware.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::AssetAddress;
use crate::error::{Result, WalletError};

const TIMEOUT: Duration = Duration::from_secs(20);

const BTC_APIS: [&str; 2] = [
    "https://mempool.space/api/address/",
    "https://blockstream.info/api/address/",
];
const LTC_APIS: [&str; 1] = ["https://litecoinspace.org/api/address/"];
const SOL_RPC: &str = "https://api.mainnet-beta.solana.com";
const TRON_API: &str = "https://api.trongrid.io/v1/accounts/";

const TRON_HOST: &str = "https://api.trongrid.io";
const PRICE_API: &str = "https://api.coingecko.com/api/v3/simple/price";

const ALL_ASSETS: [&str; 8] = ["BTC", "LTC", "XMR", "ETH", "SOL", "TRON", "USDC", "USDT"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetBalance {
    pub asset: String,

    pub minor: Option<String>,
    pub error: Option<String>,

    pub network: Option<String>,
}

impl AssetBalance {
    fn ok(asset: &str, minor: i128) -> Self {

        if minor < 0 {
            return Self::failed(
                asset,
                format!("endpoint reported a negative balance ({minor})"),
            );
        }
        Self {
            asset: asset.into(),
            minor: Some(minor.to_string()),
            error: None,
            network: None,
        }
    }

    fn failed(asset: &str, message: impl std::fmt::Display) -> Self {
        Self {
            asset: asset.into(),
            minor: None,
            error: Some(message.to_string()),
            network: None,
        }
    }

    fn ok_net(asset: &str, network: &str, minor: u128) -> Self {
        Self {
            asset: asset.into(),
            minor: Some(minor.to_string()),
            error: None,
            network: Some(network.into()),
        }
    }

    fn failed_net(asset: &str, network: &str, message: impl std::fmt::Display) -> Self {
        Self {
            asset: asset.into(),
            minor: None,
            error: Some(message.to_string()),
            network: Some(network.into()),
        }
    }
}

pub fn client() -> Result<reqwest::Client> {
    let mut builder = crate::http_client::builder()
        .timeout(TIMEOUT)

        .user_agent("Mozilla/5.0");

    if crate::tor::routing() {
        builder = builder.proxy(crate::tor::proxy()?);
    }

    builder.build().map_err(|e| WalletError::Network(e.to_string()))
}

#[cfg(not(target_os = "android"))]
pub fn plain_client() -> Result<reqwest::Client> {
    crate::http_client::builder()
        .timeout(std::time::Duration::from_secs(900))
        .user_agent("Mozilla/5.0")
        .build()
        .map_err(|e| WalletError::Network(e.to_string()))
}

fn net(e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(e.to_string())
}

async fn twice<T, F, Fut>(attempt: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    match attempt().await {
        Ok(value) => Ok(value),
        Err(first) => {
            tokio::time::sleep(Duration::from_millis(500)).await;
            attempt().await.map_err(|_| first)
        }
    }
}

pub async fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T> {
    client()?
        .get(url)
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)
}

pub async fn sol_call<T: serde::de::DeserializeOwned>(
    method: &str,
    params: serde_json::Value,
) -> Result<T> {
    let c = client()?;
    sol_rpc(&c, method, params).await
}

#[derive(Deserialize)]
struct EsploraStats {
    funded_txo_sum: i128,
    spent_txo_sum: i128,
}

#[derive(Deserialize)]
struct EsploraAddress {
    chain_stats: EsploraStats,
    mempool_stats: EsploraStats,
}

async fn esplora_any(c: &reqwest::Client, bases: &[&str], address: &str) -> Result<i128> {
    let mut last = WalletError::Network("no provider configured".into());
    for base in bases {
        match twice(|| esplora(c, base, address)).await {
            Ok(v) => return Ok(v),
            Err(e) => last = e,
        }
    }
    Err(last)
}

async fn esplora(c: &reqwest::Client, base: &str, address: &str) -> Result<i128> {
    let body: EsploraAddress = c
        .get(format!("{base}{address}"))
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;

    Ok(body.chain_stats.funded_txo_sum - body.chain_stats.spent_txo_sum
        + body.mempool_stats.funded_txo_sum
        - body.mempool_stats.spent_txo_sum)
}

#[derive(Deserialize)]
struct EsploraUtxoStatus {
    #[serde(default)]
    confirmed: bool,
}

#[derive(Deserialize)]
struct EsploraUtxo {
    txid: String,
    vout: u32,
    value: u64,
    status: EsploraUtxoStatus,
}

pub fn btc_apis(chain: super::btc_tx::Chain) -> &'static [&'static str] {
    match chain {
        super::btc_tx::Chain::Bitcoin => &BTC_APIS,
        super::btc_tx::Chain::Litecoin => &LTC_APIS,
    }
}

pub const SCAN_BATCH: u32 = 10;

pub const SCAN_GAP: u32 = 10;

pub const SCAN_CEILING: u32 = 400;

pub async fn esplora_utxos_batch(
    bases: &[&str],
    addresses: &[(u32, String)],
) -> Result<Vec<(u32, Vec<super::btc_tx::Utxo>)>> {
    let found = futures::future::join_all(addresses.iter().map(|(index, address)| async move {
        (*index, esplora_utxos(bases, address, *index).await)
    }))
    .await;

    let mut out = Vec::with_capacity(found.len());
    let mut failures = 0;
    for (index, result) in found {
        match result {
            Ok(list) => out.push((index, list)),

            Err(_) => {
                failures += 1;
                out.push((index, Vec::new()));
            }
        }
    }

    if failures == addresses.len() && !addresses.is_empty() {
        return Err(WalletError::Network(
            "no provider would report the outputs for this wallet".into(),
        ));
    }

    Ok(out)
}

pub async fn esplora_utxos(
    bases: &[&str],
    address: &str,
    key_index: u32,
) -> Result<Vec<super::btc_tx::Utxo>> {
    let mut last = WalletError::Network("no provider configured".into());
    for base in bases {
        match get_json::<Vec<EsploraUtxo>>(&format!("{base}{address}/utxo")).await {
            Ok(list) => {
                let mut out = Vec::new();
                for u in list.into_iter().filter(|u| u.status.confirmed) {
                    let raw = (0..u.txid.len())
                        .step_by(2)
                        .map(|i| u8::from_str_radix(&u.txid[i..i + 2], 16))
                        .collect::<std::result::Result<Vec<u8>, _>>()
                        .map_err(|e| WalletError::Network(format!("bad txid: {e}")))?;
                    if raw.len() != 32 {
                        return Err(WalletError::Network("txid was not 32 bytes".into()));
                    }

                    let mut txid = [0u8; 32];
                    for (i, b) in raw.iter().rev().enumerate() {
                        txid[i] = *b;
                    }
                    out.push(super::btc_tx::Utxo {
                        txid,
                        vout: u.vout,
                        value: u.value,
                        key_index,
                    });
                }
                out.sort_by(|a, b| b.value.cmp(&a.value));
                return Ok(out);
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[derive(Deserialize)]
struct EsploraChainStats {
    #[serde(default)]
    tx_count: u64,
}

#[derive(Deserialize)]
struct EsploraAddressInfo {
    chain_stats: EsploraChainStats,
    mempool_stats: EsploraChainStats,
}

pub async fn esplora_address_used(bases: &[&str], address: &str) -> Result<bool> {
    let mut last = WalletError::Network("no provider configured".into());
    for base in bases {
        match get_json::<EsploraAddressInfo>(&format!("{base}{address}")).await {
            Ok(info) => {
                return Ok(info.chain_stats.tx_count > 0 || info.mempool_stats.tx_count > 0)
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

pub async fn esplora_fee_rate(bases: &[&str]) -> Result<f64> {
    for base in bases {

        let root = base.trim_end_matches("address/");
        if let Ok(map) = get_json::<HashMap<String, f64>>(&format!("{root}fee-estimates")).await {

            if let Some(rate) = map.get("3").or_else(|| map.get("6")).or_else(|| map.get("1")) {
                return Ok(rate.max(1.0));
            }
        }
    }

    Ok(2.0)
}

pub async fn esplora_broadcast(bases: &[&str], raw: &[u8]) -> Result<String> {
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let mut last = WalletError::Network("no provider configured".into());

    for base in bases {
        let root = base.trim_end_matches("address/");
        let response = client()?
            .post(format!("{root}tx"))
            .body(hex.clone())
            .send()
            .await;

        match response {
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                if status.is_success() {
                    return Ok(body.trim().to_string());
                }

                last = WalletError::Network(format!("the network rejected it: {}", body.trim()));
            }
            Err(e) => last = net(e),
        }
    }
    Err(last)
}

#[derive(Deserialize)]
struct RpcError {
    message: String,
}

#[derive(Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

impl<T> RpcResponse<T> {
    fn take(self) -> Result<T> {
        match (self.result, self.error) {
            (Some(r), _) => Ok(r),
            (None, Some(e)) => Err(WalletError::Network(e.message)),
            (None, None) => Err(WalletError::Network("empty RPC response".into())),
        }
    }
}

async fn sol_rpc<T>(c: &reqwest::Client, method: &str, params: serde_json::Value) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let body: RpcResponse<T> = c
        .post(SOL_RPC)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": method, "params": params
        }))
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;
    body.take()
}

#[derive(Deserialize)]
struct SolValue<T> {
    value: T,
}

async fn sol_balance(c: &reqwest::Client, address: &str) -> Result<i128> {
    let v: SolValue<i128> = sol_rpc(c, "getBalance", serde_json::json!([address])).await?;
    Ok(v.value)
}

#[derive(Deserialize)]
struct TokenAmount {
    amount: String,
}

#[derive(Deserialize)]
struct TokenInfo {
    #[serde(rename = "tokenAmount")]
    token_amount: TokenAmount,
}

#[derive(Deserialize)]
struct ParsedData {
    info: TokenInfo,
}

#[derive(Deserialize)]
struct AccountData {
    parsed: ParsedData,
}

#[derive(Deserialize)]
struct TokenAccountInner {
    data: AccountData,
}

#[derive(Deserialize)]
struct TokenAccount {
    pubkey: String,
    account: TokenAccountInner,
}

pub async fn sol_token_accounts(owner: &str, mint: &str) -> Result<Vec<(String, u128)>> {
    let c = client()?;
    let v: SolValue<Vec<TokenAccount>> = sol_rpc(
        &c,
        "getTokenAccountsByOwner",
        serde_json::json!([owner, { "mint": mint }, { "encoding": "jsonParsed" }]),
    )
    .await?;

    Ok(v.value
        .into_iter()
        .filter_map(|a| {
            a.account
                .data
                .parsed
                .info
                .token_amount
                .amount
                .parse::<u128>()
                .ok()
                .map(|amt| (a.pubkey, amt))
        })
        .collect())
}

pub async fn sol_spl_balance(owner: &str, mint: &str) -> Result<u128> {
    Ok(sol_token_accounts(owner, mint)
        .await?
        .into_iter()
        .map(|(_, amt)| amt)
        .sum())
}

const ETH_RPCS: [&str; 2] = [
    "https://cloudflare-eth.com",
    "https://ethereum-rpc.publicnode.com",
];

fn parse_hex_u128(text: &str) -> Result<u128> {
    let clean = text.trim().trim_start_matches("0x");
    if clean.is_empty() {
        return Ok(0);
    }
    u128::from_str_radix(clean, 16)
        .map_err(|e| WalletError::Network(format!("bad quantity {text}: {e}")))
}

async fn eth_call_rpc<T: serde::de::DeserializeOwned>(
    method: &str,
    params: serde_json::Value,
) -> Result<T> {
    let c = client()?;
    let mut last = WalletError::Network("no endpoint configured".into());

    for url in ETH_RPCS {
        let sent = c
            .post(url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params
            }))
            .send()
            .await;

        match sent {
            Ok(r) => match r.error_for_status() {
                Ok(ok) => match ok.json::<RpcResponse<T>>().await {
                    Ok(body) => match body.take() {
                        Ok(value) => return Ok(value),
                        Err(e) => last = e,
                    },
                    Err(e) => last = net(e),
                },
                Err(e) => last = net(e),
            },
            Err(e) => last = net(e),
        }
    }
    Err(last)
}

pub async fn eth_balance(address: &str) -> Result<u128> {
    let hex: String =
        eth_call_rpc("eth_getBalance", serde_json::json!([address, "latest"])).await?;
    parse_hex_u128(&hex)
}

pub async fn eth_nonce(address: &str) -> Result<u64> {
    let hex: String =
        eth_call_rpc("eth_getTransactionCount", serde_json::json!([address, "pending"])).await?;
    Ok(parse_hex_u128(&hex)? as u64)
}

#[derive(Deserialize)]
struct EthBlock {
    #[serde(rename = "baseFeePerGas", default)]
    base_fee: Option<String>,
}

pub async fn eth_fees() -> Result<(u128, u128)> {
    let block: EthBlock =
        eth_call_rpc("eth_getBlockByNumber", serde_json::json!(["latest", false])).await?;
    let base = match block.base_fee {
        Some(hex) => parse_hex_u128(&hex)?,
        None => 0,
    };

    let tip = match eth_call_rpc::<String>("eth_maxPriorityFeePerGas", serde_json::json!([])).await
    {
        Ok(hex) => parse_hex_u128(&hex).unwrap_or(1_500_000_000),

        Err(_) => 1_500_000_000,
    };

    Ok((base * 2 + tip, tip))
}

#[allow(dead_code)]
pub async fn eth_estimate_gas(from: &str, to: &str, data: &str) -> Result<u64> {
    let hex: String = eth_call_rpc(
        "eth_estimateGas",
        serde_json::json!([{ "from": from, "to": to, "data": data }]),
    )
    .await?;

    Ok((parse_hex_u128(&hex)? as u64).saturating_mul(12) / 10)
}

#[allow(dead_code)]
pub async fn eth_view(contract: &str, data: &str) -> Result<u128> {
    let hex: String = eth_call_rpc(
        "eth_call",
        serde_json::json!([{ "to": contract, "data": data }, "latest"]),
    )
    .await?;
    parse_hex_u128(&hex)
}

pub async fn eth_broadcast(raw: &[u8]) -> Result<String> {
    let hex: String = format!("0x{}", raw.iter().map(|b| format!("{b:02x}")).collect::<String>());
    eth_call_rpc("eth_sendRawTransaction", serde_json::json!([hex])).await
}

#[derive(Deserialize)]
struct TronAccount {
    #[serde(default)]
    balance: i128,
    #[serde(default)]
    trc20: Vec<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct TronResponse {
    #[serde(default)]
    data: Vec<TronAccount>,
}

async fn tron_account(c: &reqwest::Client, address: &str) -> Result<Option<TronAccount>> {
    let body: TronResponse = c
        .get(format!("{TRON_API}{address}"))
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;
    Ok(body.data.into_iter().next())
}

pub async fn tron_trx_balance(address: &str) -> Result<u64> {
    let c = client()?;
    match tron_account(&c, address).await? {
        Some(a) => Ok(a.balance.max(0) as u64),
        None => Ok(0),
    }
}

pub async fn tron_trc20_balance(address: &str, contract: &str) -> Result<u128> {
    let c = client()?;
    match tron_account(&c, address).await? {
        Some(a) => Ok(a
            .trc20
            .iter()
            .filter_map(|m| m.get(contract))
            .filter_map(|v| v.parse::<u128>().ok())
            .sum()),
        None => Ok(0),
    }
}

fn tron_message(v: &serde_json::Value) -> String {
    let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("");
    let raw = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
    let decoded = super::tron_tx::unhex(raw)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .filter(|s| s.chars().all(|c| !c.is_control()) && !s.is_empty())
        .unwrap_or_else(|| raw.to_string());
    format!("{code} {decoded}").trim().to_string()
}

pub async fn tron_create_transfer(
    owner: &str,
    to: &str,
    amount: u64,
) -> Result<serde_json::Value> {
    let c = client()?;
    let body = serde_json::json!({
        "owner_address": owner,
        "to_address": to,
        "amount": amount,
        "visible": true,
    });
    let v: serde_json::Value = c
        .post(format!("{TRON_HOST}/wallet/createtransaction"))
        .json(&body)
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;

    if let Some(err) = v.get("Error").and_then(|e| e.as_str()) {
        return Err(net(format!("Tron: {err}")));
    }
    if v.get("txID").is_none() {
        return Err(net("Tron did not build the transaction"));
    }
    Ok(v)
}

pub async fn tron_trigger(
    owner: &str,
    contract: &str,
    selector: &str,
    parameter_hex: &str,
    fee_limit: u64,
) -> Result<serde_json::Value> {
    let c = client()?;
    let body = serde_json::json!({
        "owner_address": owner,
        "contract_address": contract,
        "function_selector": selector,
        "parameter": parameter_hex,
        "fee_limit": fee_limit,
        "call_value": 0,
        "visible": true,
    });
    let v: serde_json::Value = c
        .post(format!("{TRON_HOST}/wallet/triggersmartcontract"))
        .json(&body)
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;

    let ok = v
        .get("result")
        .and_then(|r| r.get("result"))
        .and_then(|r| r.as_bool())
        .unwrap_or(false);
    if !ok {
        let msg = v
            .get("result")
            .map(tron_message)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "the contract call could not be built".into());
        return Err(net(format!("Tron: {msg}")));
    }
    v.get("transaction")
        .cloned()
        .ok_or_else(|| net("Tron did not build the transaction"))
}

pub async fn tron_broadcast(signed: &serde_json::Value) -> Result<String> {
    let c = client()?;
    let v: serde_json::Value = c
        .post(format!("{TRON_HOST}/wallet/broadcasttransaction"))
        .json(signed)
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;

    if v.get("result").and_then(|r| r.as_bool()).unwrap_or(false) {
        let id = v
            .get("txid")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();
        return Ok(id);
    }
    Err(net(format!("Tron rejected the transfer: {}", tron_message(&v))))
}

async fn eth_token_balance(address: &str, contract: &str) -> Result<u128> {
    let owner = super::eth::parse_address(address)?;
    let data: String = super::eth::erc20_balance_data(&owner)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    eth_view(contract, &format!("0x{data}")).await
}

async fn token_balances(
    sol: Option<&str>,
    eth: Option<&str>,
    tron: Option<&str>,
) -> Vec<AssetBalance> {
    use super::tokens::contract as con;

    async fn spl_bal(a: Option<&str>, mint: &str) -> Result<u128> {
        match a {
            Some(a) => sol_spl_balance(a, mint).await,
            None => Err(WalletError::Network("no Solana address".into())),
        }
    }
    async fn erc_bal(a: Option<&str>, c: &str) -> Result<u128> {
        match a {
            Some(a) => eth_token_balance(a, c).await,
            None => Err(WalletError::Network("no Ethereum address".into())),
        }
    }
    async fn trc_bal(a: Option<&str>, c: &str) -> Result<u128> {
        match a {
            Some(a) => tron_trc20_balance(a, c).await,
            None => Err(WalletError::Network("no Tron address".into())),
        }
    }

    let known = |asset, net| con(asset, net).expect("known token contract");
    let (uc_s, uc_e, uc_t, ut_s, ut_e, ut_t) = tokio::join!(
        spl_bal(sol, known("USDC", "SOL")),
        erc_bal(eth, known("USDC", "ETH")),
        trc_bal(tron, known("USDC", "TRON")),
        spl_bal(sol, known("USDT", "SOL")),
        erc_bal(eth, known("USDT", "ETH")),
        trc_bal(tron, known("USDT", "TRON")),
    );

    let mut out = Vec::with_capacity(8);
    for (asset, cells) in [
        ("USDC", [("SOL", uc_s), ("ETH", uc_e), ("TRON", uc_t)]),
        ("USDT", [("SOL", ut_s), ("ETH", ut_e), ("TRON", ut_t)]),
    ] {
        let mut total: u128 = 0;
        let mut any_ok = false;
        let mut last_err: Option<String> = None;
        for (net, res) in cells {
            match res {
                Ok(v) => {
                    out.push(AssetBalance::ok_net(asset, net, v));
                    total += v;
                    any_ok = true;
                }
                Err(e) => {
                    let msg = e.to_string();
                    out.push(AssetBalance::failed_net(asset, net, &msg));
                    last_err = Some(msg);
                }
            }
        }

        out.push(if any_ok {
            AssetBalance {
                asset: asset.into(),
                minor: Some(total.to_string()),
                error: last_err,
                network: None,
            }
        } else {
            AssetBalance::failed(asset, last_err.unwrap_or_else(|| "unavailable".into()))
        });
    }
    out
}

fn address_of(list: &[AssetAddress], asset: &str) -> Option<String> {
    list.iter()
        .find(|a| a.asset == asset)
        .and_then(|a| a.address.clone())
}

pub async fn balances(addresses: &[AssetAddress]) -> Vec<AssetBalance> {
    let c = match client() {
        Ok(c) => c,
        Err(e) => {
            return ALL_ASSETS
                .iter()
                .map(|a| AssetBalance::failed(a, &e))
                .collect()
        }
    };

    let btc_addr = address_of(addresses, "BTC");
    let ltc_addr = address_of(addresses, "LTC");
    let eth_addr = address_of(addresses, "ETH");
    let sol_addr = address_of(addresses, "SOL");
    let tron_addr = address_of(addresses, "TRON");

    let missing = || WalletError::Network("no address".into());

    let (btc, ltc, eth, sol, tron) = tokio::join!(
        async {
            match &btc_addr {
                Some(a) => esplora_any(&c, &BTC_APIS, a).await,
                None => Err(missing()),
            }
        },
        async {
            match &ltc_addr {
                Some(a) => esplora_any(&c, &LTC_APIS, a).await,
                None => Err(missing()),
            }
        },
        async {
            match &eth_addr {
                Some(a) => twice(|| async { eth_balance(a).await.map(|v| v as i128) }).await,
                None => Err(missing()),
            }
        },
        async {
            match &sol_addr {
                Some(a) => twice(|| sol_balance(&c, a)).await,
                None => Err(missing()),
            }
        },
        async {
            match &tron_addr {
                Some(a) => twice(|| tron_account(&c, a)).await,
                None => Err(missing()),
            }
        },
    );

    let one = |asset: &str, result: Result<i128>| match result {
        Ok(v) => AssetBalance::ok(asset, v),
        Err(e) => AssetBalance::failed(asset, e),
    };

    let mut out = Vec::with_capacity(ALL_ASSETS.len());
    out.push(one("BTC", btc));
    out.push(one("LTC", ltc));

    out.push(AssetBalance::failed(
        "XMR",
        "Monero cannot be queried by address. Reading this balance needs a node          or light-wallet server scanning with your private view key.",
    ));

    out.push(one("ETH", eth));
    out.push(one("SOL", sol));

    match tron {

        Ok(None) => out.push(AssetBalance::ok("TRON", 0)),
        Ok(Some(account)) => out.push(AssetBalance::ok("TRON", account.balance)),
        Err(e) => out.push(AssetBalance::failed("TRON", &e)),
    }

    out.extend(
        token_balances(
            sol_addr.as_deref(),
            eth_addr.as_deref(),
            tron_addr.as_deref(),
        )
        .await,
    );
    out
}

#[derive(Deserialize)]
struct Blockhash {
    blockhash: String,
}

pub async fn sol_latest_blockhash() -> Result<[u8; 32]> {
    let c = client()?;
    let v: SolValue<Blockhash> = sol_rpc(
        &c,
        "getLatestBlockhash",
        serde_json::json!([{ "commitment": "finalized" }]),
    )
    .await?;

    let raw = bs58::decode(&v.value.blockhash)
        .into_vec()
        .map_err(|e| WalletError::Network(format!("bad blockhash: {e}")))?;
    if raw.len() != 32 {
        return Err(WalletError::Network("blockhash was not 32 bytes".into()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&raw);
    Ok(out)
}

#[derive(Deserialize)]
struct SimulationValue {
    err: Option<serde_json::Value>,
    #[serde(default)]
    logs: Option<Vec<String>>,
}

pub async fn sol_simulate(tx: &[u8]) -> Result<()> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(tx);

    let c = client()?;
    let v: SolValue<SimulationValue> = sol_rpc(
        &c,
        "simulateTransaction",
        serde_json::json!([encoded, { "encoding": "base64", "sigVerify": true }]),
    )
    .await?;

    match v.value.err {
        None => Ok(()),
        Some(e) => {

            let detail = v
                .value
                .logs
                .and_then(|l| l.last().cloned())
                .unwrap_or_default();
            Err(WalletError::Network(if detail.is_empty() {
                format!("the network rejected this transfer: {e}")
            } else {
                format!("the network rejected this transfer: {e} ({detail})")
            }))
        }
    }
}

pub async fn sol_rent_exempt_minimum() -> Result<u64> {
    let c = client()?;
    sol_rpc(&c, "getMinimumBalanceForRentExemption", serde_json::json!([0])).await
}

pub async fn sol_balance_of(address: &str) -> Result<i128> {
    let c = client()?;
    sol_balance(&c, address).await
}

#[cfg(test)]
pub async fn sol_fee_for_message(message: &[u8]) -> Result<Option<u64>> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(message);

    let c = client()?;
    let v: SolValue<Option<u64>> = sol_rpc(
        &c,
        "getFeeForMessage",
        serde_json::json!([encoded, { "commitment": "processed" }]),
    )
    .await?;
    Ok(v.value)
}

pub async fn sol_broadcast(tx: &[u8]) -> Result<String> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(tx);

    let c = client()?;
    sol_rpc(
        &c,
        "sendTransaction",
        serde_json::json!([encoded, { "encoding": "base64" }]),
    )
    .await
}

const PRICE_IDS: [(&str, &str); 8] = [
    ("BTC", "bitcoin"),
    ("LTC", "litecoin"),
    ("XMR", "monero"),
    ("ETH", "ethereum"),
    ("SOL", "solana"),
    ("TRON", "tron"),
    ("USDC", "usd-coin"),
    ("USDT", "tether"),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub price: f64,
    pub change24h: Option<f64>,
}

pub const CURRENCIES: [&str; 8] = ["usd", "eur", "gbp", "chf", "jpy", "cad", "aud", "sek"];

pub async fn prices(currency: &str) -> Result<HashMap<String, Quote>> {
    let currency = currency.to_ascii_lowercase();
    if !CURRENCIES.contains(&currency.as_str()) {
        return Err(WalletError::Network(format!("unsupported currency: {currency}")));
    }

    let ids = PRICE_IDS
        .iter()
        .map(|(_, id)| *id)
        .collect::<Vec<_>>()
        .join(",");

    let url = format!(
        "{PRICE_API}?ids={ids}&vs_currencies={currency}&include_24hr_change=true"
    );

    let raw: HashMap<String, HashMap<String, f64>> = client()?
        .get(url)
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .json()
        .await
        .map_err(net)?;

    let change_key = format!("{currency}_24h_change");

    Ok(PRICE_IDS
        .iter()
        .filter_map(|(asset, id)| {
            let entry = raw.get(*id)?;
            let price = *entry.get(&currency)?;
            Some((
                (*asset).to_string(),
                Quote {
                    price,
                    change24h: entry.get(&change_key).copied(),
                },
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdc_mint_is_a_valid_solana_pubkey() {
        let mint = crate::chains::tokens::contract("USDC", "SOL").unwrap();
        let raw = bs58::decode(mint).into_vec().expect("valid base58");
        assert_eq!(raw.len(), 32, "a Solana mint is a 32 byte public key");
    }

    #[test]
    fn usdt_contract_is_a_valid_tron_address() {
        let contract = crate::chains::tokens::contract("USDT", "TRON").unwrap();
        let raw: Vec<u8> = bs58::decode(contract)
            .with_check(None)
            .into_vec()
            .expect("valid base58check");
        assert_eq!(raw.len(), 21);
        assert_eq!(raw[0], 0x41, "Tron addresses carry the 0x41 tag");
    }

    #[test]
    fn every_asset_gets_a_row_even_with_no_addresses() {

        let out = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { balances(&[]).await });

        let names: Vec<&str> = out.iter().map(|b| b.asset.as_str()).collect();
        for asset in ALL_ASSETS {
            assert!(names.contains(&asset), "{asset} missing from the result");
        }

        assert!(out.iter().all(|b| b.minor.is_none() && b.error.is_some()));
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::crypto::seed;

    fn reference_addresses() -> Vec<AssetAddress> {
        let phrase = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";
        let s = *seed::to_seed(&seed::parse(phrase).unwrap());
        super::super::addresses(&s).unwrap()
    }

    #[tokio::test]
    #[ignore]
    async fn live_balances_parse() {
        let out = balances(&reference_addresses()).await;
        for b in &out {
            match (&b.minor, &b.error) {
                (Some(v), _) => println!("{:5} {v}", b.asset),
                (None, Some(e)) => println!("{:5} error: {e}", b.asset),
                (None, None) => panic!("{} returned neither a value nor a reason", b.asset),
            }
        }

        for b in out.iter().filter(|b| b.asset != "XMR") {
            assert!(b.minor.is_some(), "{} failed: {:?}", b.asset, b.error);
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_solana_transfer_simulates() {
        use crate::chains::sol_tx;

        let phrase = "abandon abandon abandon abandon abandon abandon                       abandon abandon abandon abandon abandon about";
        let s = *seed::to_seed(&seed::parse(phrase).unwrap());

        let blockhash = sol_latest_blockhash().await.expect("blockhash");
        let tx = sol_tx::signed_transfer(
            &s,
            "1nc1nerator11111111111111111111111111111111",
            1_000_000,
            &blockhash,
        )
        .expect("signed transfer");

        println!("transaction is {} bytes", tx.len());

        let message = sol_tx::build_message(
            &sol_tx::signing_key(&s).verifying_key().to_bytes(),
            &sol_tx::parse_address("1nc1nerator11111111111111111111111111111111").unwrap(),
            1_000_000,
            &blockhash,
        );
        let fee = sol_fee_for_message(&message)
            .await
            .expect("fee lookup")
            .expect("a well formed message has a price");
        println!("mainnet priced the message at {fee} lamports");
        assert_eq!(fee, sol_tx::LAMPORTS_PER_SIGNATURE);

        match sol_simulate(&tx).await {
            Ok(()) => println!("mainnet simulated the transfer without error"),
            Err(e) => {
                let text = e.to_string();
                assert!(
                    text.contains("InvalidAccountForFee"),
                    "unexpected rejection: {text}"
                );
                println!("signature and message accepted; payer cannot cover fees, as expected");
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn live_history_parses() {
        use crate::chains::history;

        let entries = history::all(&reference_addresses()).await;
        println!("{} entries", entries.len());
        for e in entries.iter().take(8) {
            println!(
                "{:5} {:3} {:>16} {:?} confirmed={}",
                e.asset, e.direction, e.amount_minor, e.timestamp, e.confirmed
            );
        }

        assert!(!entries.is_empty(), "these addresses have known history");
        for e in &entries {
            assert!(e.direction == "in" || e.direction == "out");
            assert!(e.amount_minor.parse::<u128>().is_ok(), "bad amount: {}", e.amount_minor);
            assert!(!e.id.is_empty());
        }

        let times: Vec<i64> = entries.iter().filter_map(|e| e.timestamp).collect();
        assert!(times.windows(2).all(|w| w[0] >= w[1]), "not sorted newest first");
    }

    #[tokio::test]
    #[ignore]
    async fn live_prices_cover_every_asset() {
        let p = prices("eur").await.expect("price feed should answer");
        for (asset, _) in PRICE_IDS {
            let q = p.get(asset).unwrap_or_else(|| panic!("{asset} missing"));
            println!("{asset:5} {:>12.4} EUR  24h {:?}", q.price, q.change24h);
            assert!(q.price > 0.0);
        }
    }
}

#[cfg(test)]
mod live_eth {
    use super::*;
    use crate::chains::eth;
    use crate::crypto::seed;

    #[tokio::test]
    #[ignore]
    async fn live_ethereum_reads() {
        let phrase = "abandon abandon abandon abandon abandon abandon \
                      abandon abandon abandon abandon abandon about";
        let s = *seed::to_seed(&seed::parse(phrase).unwrap());
        let keys = eth::keys(&s).unwrap();
        let address = eth::to_checksum(&keys.address);
        println!("address {address}");

        let balance = eth_balance(&address).await.expect("balance");
        println!("balance {balance} wei");

        let nonce = eth_nonce(&address).await.expect("nonce");
        println!("nonce   {nonce}");

        let (max_fee, tip) = eth_fees().await.expect("fees");
        println!("max fee {max_fee} wei, tip {tip} wei");
        assert!(max_fee > tip, "the cap must exceed the tip");
        assert!(max_fee < 10_000_000_000_000u128, "fee looks implausible");

        let cost = max_fee * eth::TRANSFER_GAS as u128;
        println!("transfer ceiling {} ETH", cost as f64 / 1e18);
        assert!(cost > 0);
    }
}
