//! Balance and price lookups.
//!
//! This is the only part of the wallet that touches the network, and every
//! request here tells a third party which addresses you are interested in and
//! what your IP is. That is a real privacy cost, it is surfaced in the UI, and
//! these endpoints are meant to become configurable once the connectivity
//! section of the spec is settled.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::AssetAddress;
use crate::error::{Result, WalletError};

const TIMEOUT: Duration = Duration::from_secs(20);

// Several providers speak the same Esplora API, so each chain lists more than
// one. mempool.space in particular refuses connections often enough from some
// networks that a single source is not dependable.
const BTC_APIS: [&str; 2] = [
    "https://mempool.space/api/address/",
    "https://blockstream.info/api/address/",
];
const LTC_APIS: [&str; 1] = ["https://litecoinspace.org/api/address/"];
const SOL_RPC: &str = "https://api.mainnet-beta.solana.com";
const TRON_API: &str = "https://api.trongrid.io/v1/accounts/";
/// Full-node HTTP API, for building and broadcasting Tron transactions.
const TRON_HOST: &str = "https://api.trongrid.io";
const PRICE_API: &str = "https://api.coingecko.com/api/v3/simple/price";

const ALL_ASSETS: [&str; 8] = ["BTC", "LTC", "XMR", "ETH", "SOL", "TRON", "USDC", "USDT"];

/// One asset's balance, or the reason it could not be read. A failure on one
/// chain must not blank out the others.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetBalance {
    pub asset: String,
    /// Smallest units as a decimal string. Never a number: a JS float cannot
    /// hold 12-decimal Monero amounts without losing the low digits.
    pub minor: Option<String>,
    pub error: Option<String>,
    /// For a token, which network this balance is on. `None` for a native coin
    /// or for a token's summed-across-networks total.
    pub network: Option<String>,
}

impl AssetBalance {
    fn ok(asset: &str, minor: i128) -> Self {
        // A negative balance is arithmetically impossible and means the
        // endpoint or our parsing of it is wrong. Clamping to zero would hide
        // that behind a plausible-looking number, so it is reported instead.
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

    /// A token balance on a specific network.
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

/// The client every remote lookup uses.
///
/// When Tor routing is on, requests go through the local Tor proxy, so the
/// endpoint sees a Tor exit rather than this machine. When it is off, they go
/// out directly. Tor is checked per call, so the toggle takes effect at once.
pub fn client() -> Result<reqwest::Client> {
    let mut builder = crate::http_client::builder()
        .timeout(TIMEOUT)
        // Deliberately generic. Announcing the wallet by name in every request
        // would hand the endpoint operator an easy fingerprint.
        .user_agent("Mozilla/5.0");

    if crate::tor::routing() {
        builder = builder.proxy(crate::tor::proxy()?);
    }

    builder.build().map_err(|e| WalletError::Network(e.to_string()))
}

/// A client that never goes through Tor, whatever the setting.
///
/// Used to fetch Tor itself, since the proxy cannot carry the download that
/// installs it, and for the local Monero download where a large transfer over
/// Tor would be needlessly slow. Android downloads neither — both come inside
/// the APK — so it has no use there.
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

/// Runs a request, and on failure gives it one more go after a short pause.
///
/// These endpoints drop the occasional connection, and a single blip should
/// not blank a balance the user was reading a moment ago. The first error is
/// what gets reported if the retry fails too, since it is usually the more
/// informative one.
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

/// Plain GET returning parsed JSON. Shared with the history module.
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

/// One Solana JSON-RPC call, for callers outside this module.
pub async fn sol_call<T: serde::de::DeserializeOwned>(
    method: &str,
    params: serde_json::Value,
) -> Result<T> {
    let c = client()?;
    sol_rpc(&c, method, params).await
}

// ---------- Bitcoin and Litecoin, via the Esplora API shape ----------

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

/// Tries each provider in turn, returning the first that answers.
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

    // Unconfirmed movement is included so a just-received payment shows up
    // rather than appearing to vanish until the next block.
    Ok(body.chain_stats.funded_txo_sum - body.chain_stats.spent_txo_sum
        + body.mempool_stats.funded_txo_sum
        - body.mempool_stats.spent_txo_sum)
}

// ---------- Bitcoin and Litecoin spending ----------

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

/// Confirmed spendable outputs, largest first.
///
/// Unconfirmed ones are skipped: spending an output that has not settled makes
/// the new transaction depend on it, and a wallet this young should not be
/// building chains of unconfirmed spends.
/// Addresses fetched per round. Esplora has no multi-address endpoint, so
/// this is one request each.
pub const SCAN_BATCH: u32 = 10;

/// Stop after this many consecutive addresses hold nothing. An unfragmented
/// wallet therefore costs one batch, and a heavily fragmented one costs
/// roughly as many requests as it has used addresses.
pub const SCAN_GAP: u32 = 10;

/// Hard stop, so a scan can never run away.
pub const SCAN_CEILING: u32 = 400;

/// Outputs for one batch of addresses, kept per address so the caller can
/// tell where the used range ends.
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
            // One address failing must not hide the rest, but if every one
            // fails the caller must not conclude the wallet is empty.
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
                    // The API prints ids in display order; the wire format is
                    // the reverse.
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

/// Whether an address has ever appeared on the chain.
///
/// Reusing an address links every payment ever sent to it, so receiving
/// hands out the first one that has never been seen. Mempool activity counts
/// as used, otherwise a second receive within a block would reuse it.
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

/// Fee rate in satoshi per virtual byte for confirmation within a few blocks.
pub async fn esplora_fee_rate(bases: &[&str]) -> Result<f64> {
    for base in bases {
        // The estimates endpoint sits beside the address one.
        let root = base.trim_end_matches("address/");
        if let Ok(map) = get_json::<HashMap<String, f64>>(&format!("{root}fee-estimates")).await {
            // Three blocks is a reasonable default: not the most expensive
            // tier, but not an overnight wait either.
            if let Some(rate) = map.get("3").or_else(|| map.get("6")).or_else(|| map.get("1")) {
                return Ok(rate.max(1.0));
            }
        }
    }
    // Every provider refused, so fall back to the network minimum rather than
    // failing the whole send.
    Ok(2.0)
}

/// Publishes a signed transaction. Returns its id.
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
                // The node explains why it refused, and that message is far
                // more useful than the status code.
                last = WalletError::Network(format!("the network rejected it: {}", body.trim()));
            }
            Err(e) => last = net(e),
        }
    }
    Err(last)
}

// ---------- Solana ----------

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

/// The owner's token accounts for a mint, as (account pubkey, amount) pairs.
/// The pubkey is the on-chain address that actually holds the tokens, read
/// from the node rather than derived, so it can be trusted as ground truth
/// and cross-checked against a local derivation.
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

/// An SPL token balance in its smallest unit, summed across the owner's
/// accounts for that mint.
pub async fn sol_spl_balance(owner: &str, mint: &str) -> Result<u128> {
    Ok(sol_token_accounts(owner, mint)
        .await?
        .into_iter()
        .map(|(_, amt)| amt)
        .sum())
}

// ---------- Ethereum ----------

/// Public endpoints, tried in order. Both were checked to answer eth_chainId
/// with mainnet; several other well-known ones now require an API key.
const ETH_RPCS: [&str; 2] = [
    "https://cloudflare-eth.com",
    "https://ethereum-rpc.publicnode.com",
];

/// Quantities come back as minimal hex with an 0x prefix.
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

/// Next usable nonce. "pending" is used so a second send does not collide
/// with one still sitting in the mempool.
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

/// Current base fee and a tip, in wei.
///
/// The cap is set to twice the base fee plus the tip, which is the usual
/// headroom: the base fee can rise at most 12.5% per block, so this survives
/// several blocks of congestion without overpaying, since anything unused is
/// refunded.
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
        // Not every endpoint implements it; 1.5 gwei is the common default.
        Err(_) => 1_500_000_000,
    };

    Ok((base * 2 + tip, tip))
}

/// Gas an ERC-20 transfer will take. Estimated rather than assumed,
/// because it depends on whether the recipient already holds the token.
///
/// Unused until USDT on Ethereum is wired up, which is what it exists for.
#[allow(dead_code)]
pub async fn eth_estimate_gas(from: &str, to: &str, data: &str) -> Result<u64> {
    let hex: String = eth_call_rpc(
        "eth_estimateGas",
        serde_json::json!([{ "from": from, "to": to, "data": data }]),
    )
    .await?;
    // A little headroom, since the estimate is taken against the current
    // state and execution happens later.
    Ok((parse_hex_u128(&hex)? as u64).saturating_mul(12) / 10)
}

/// Reads a contract without spending anything.
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

// ---------- Tron ----------

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

/// TRX balance in sun, or zero for an account that has never been funded.
pub async fn tron_trx_balance(address: &str) -> Result<u64> {
    let c = client()?;
    match tron_account(&c, address).await? {
        Some(a) => Ok(a.balance.max(0) as u64),
        None => Ok(0),
    }
}

/// A TRC-20 token balance in its smallest unit, for one contract.
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

/// A Tron error body that surfaced as `message` is often hex-encoded ASCII.
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

/// Asks the node to build an unsigned TRX transfer. The node supplies the
/// block reference and timestamps; the caller verifies the recipient and
/// amount against the returned raw data before signing it.
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

/// Asks the node to build an unsigned TRC-20 (or other) contract call. Same
/// trust model as `tron_create_transfer`: the raw data is verified before it
/// is signed.
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

    // triggersmartcontract wraps the tx and reports a result block.
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

/// Broadcasts a signed Tron transaction, returning its id on success.
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

/// An ERC-20 token balance for an address, via `balanceOf`.
async fn eth_token_balance(address: &str, contract: &str) -> Result<u128> {
    let owner = super::eth::parse_address(address)?;
    let data: String = super::eth::erc20_balance_data(&owner)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    eth_view(contract, &format!("0x{data}")).await
}

/// Every stablecoin balance: one entry per network, plus a summed total per
/// token (the total carries no network). All six network reads run at once.
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
        // The summed total, so a wallet holding the same coin on two chains
        // shows one figure as well as the split. Carries the last error if any
        // network could not be read, so a partial total is not read as final.
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

// ---------- Entry point ----------

fn address_of(list: &[AssetAddress], asset: &str) -> Option<String> {
    list.iter()
        .find(|a| a.asset == asset)
        .and_then(|a| a.address.clone())
}

/// Reads every chain at once.
///
/// These were sequential once, which meant a single slow or timing-out
/// endpoint held up all the others and left the UI showing "Refreshing" for
/// most of a minute. Now the whole set takes as long as the slowest one.
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

    // Monero balances cannot be looked up from an address. Outputs are only
    // discoverable by scanning the chain with the account private view key,
    // which needs a node or a light-wallet server, not a REST call.
    out.push(AssetBalance::failed(
        "XMR",
        "Monero cannot be queried by address. Reading this balance needs a node          or light-wallet server scanning with your private view key.",
    ));

    out.push(one("ETH", eth));
    out.push(one("SOL", sol));

    match tron {
        // An account that has never been funded does not exist on Tron, which
        // is a real zero rather than an error.
        Ok(None) => out.push(AssetBalance::ok("TRON", 0)),
        Ok(Some(account)) => out.push(AssetBalance::ok("TRON", account.balance)),
        Err(e) => out.push(AssetBalance::failed("TRON", &e)),
    }

    // Every stablecoin, per network plus a total. These reach the token
    // contracts on each chain, so they run after the native reads above.
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

// ---------- Solana transfers ----------

#[derive(Deserialize)]
struct Blockhash {
    blockhash: String,
}

/// A transaction is only valid against a recent blockhash, which is also what
/// stops it being replayed later.
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

/// Runs the transaction against a node without submitting it. The node
/// executes it exactly as it would on chain and reports what would happen,
/// which is how a transfer can be checked without spending anything.
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
            // The last log line is usually the useful one.
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

/// Smallest balance a zero-data account may keep without being purged.
///
/// Solana will not let a transfer leave an account holding less than this
/// unless it empties it completely, so any send has to either stay under
/// balance minus fee minus this, or take everything.
pub async fn sol_rent_exempt_minimum() -> Result<u64> {
    let c = client()?;
    sol_rpc(&c, "getMinimumBalanceForRentExemption", serde_json::json!([0])).await
}

/// Current balance in lamports for one address.
pub async fn sol_balance_of(address: &str) -> Result<i128> {
    let c = client()?;
    sol_balance(&c, address).await
}

/// Asks a node what this message would cost. It has to deserialize the
/// message to answer, so a non-null result also confirms the encoding is
/// well formed, independently of whether the payer can afford it.
///
/// Only the transaction-format test needs this; the send flow uses the fixed
/// per-signature fee instead of paying for an extra round trip.
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

/// Submits the transaction. Returns its signature, which is its id on chain.
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

// ---------- Prices ----------

/// CoinGecko ids for each asset.
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

/// A spot price plus its move over the last day, in whatever currency was
/// asked for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quote {
    pub price: f64,
    pub change24h: Option<f64>,
}

/// Currencies the price feed will answer in. Kept to a fixed list so a bad
/// value cannot be pushed into the request.
pub const CURRENCIES: [&str; 8] = ["usd", "eur", "gbp", "chf", "jpy", "cad", "aud", "sek"];

/// Spot prices in USD, keyed by asset id. One request covers all seven, so the
/// price feed learns nothing beyond the fact that this IP wants crypto prices.
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

    // Ids and currency are both from fixed lists, so nothing needs escaping.
    let url = format!(
        "{PRICE_API}?ids={ids}&vs_currencies={currency}&include_24hr_change=true"
    );

    // The response keys carry the currency in them, so it is read as a plain
    // map rather than a fixed struct.
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

    // These two constants decide which token a balance is read for. A typo
    // does not fail loudly at runtime, it just reports zero forever, so the
    // shape is checked here instead.

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
        // A caller must always be able to render one line per asset.
        let out = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { balances(&[]).await });

        let names: Vec<&str> = out.iter().map(|b| b.asset.as_str()).collect();
        for asset in ALL_ASSETS {
            assert!(names.contains(&asset), "{asset} missing from the result");
        }
        // Nothing can have a balance without an address.
        assert!(out.iter().all(|b| b.minor.is_none() && b.error.is_some()));
    }
}

/// Live network checks. Ignored by default so an offline machine or a rate
/// limit never fails an ordinary test run. Run with:
///   cargo test live_ -- --ignored --nocapture
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
        // Everything except Monero should have answered with a number.
        for b in out.iter().filter(|b| b.asset != "XMR") {
            assert!(b.minor.is_some(), "{} failed: {:?}", b.asset, b.error);
        }
    }

    /// Builds and signs a real transfer from the reference account, then has
    /// a mainnet node execute it in simulation. Nothing is submitted and no
    /// funds move, but the node verifies the signature and the serialized
    /// message, so this fails loudly if either is wrong.
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

        // The node has to deserialize the message to price it, so a real fee
        // back means the encoding is right.
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

        // Simulation goes further and verifies the signature as well. This
        // particular reference account has been assigned to a third-party
        // program, so it cannot pay fees and the node says so. Reaching that
        // specific complaint means the message parsed and the signature
        // checked out; anything else is a real defect.
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

    /// Pulls real history for the reference addresses and checks every entry
    /// came back coherent. Catches a shape change in any of the four APIs.
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

        // Newest first.
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

    /// Reads the reference account from mainnet and checks the fee market
    /// answers with sane numbers. Ignored by default like the other live
    /// tests.
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

        // A plain transfer at this rate, in ether.
        let cost = max_fee * eth::TRANSFER_GAS as u128;
        println!("transfer ceiling {} ETH", cost as f64 / 1e18);
        assert!(cost > 0);
    }
}
