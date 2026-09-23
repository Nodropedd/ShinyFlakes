//! Transaction history.

use serde::{Deserialize, Serialize};

use super::rpc;
use super::AssetAddress;
use crate::error::{Result, WalletError};

const LIMIT: usize = 15;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub asset: String,
    pub id: String,

    pub direction: String,

    pub amount_minor: String,

    pub timestamp: Option<i64>,
    pub confirmed: bool,
}

impl Entry {
    fn new(asset: &str, id: String, net: i128, timestamp: Option<i64>, confirmed: bool) -> Self {
        Self {
            asset: asset.into(),
            id,
            direction: if net >= 0 { "in" } else { "out" }.into(),
            amount_minor: net.abs().to_string(),
            timestamp,
            confirmed,
        }
    }
}

#[derive(Deserialize)]
struct EsploraOut {
    #[serde(default)]
    scriptpubkey_address: Option<String>,
    #[serde(default)]
    value: i128,
}

#[derive(Deserialize)]
struct EsploraIn {
    #[serde(default)]
    prevout: Option<EsploraOut>,
}

#[derive(Deserialize)]
struct EsploraStatus {
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    block_time: Option<i64>,
}

#[derive(Deserialize)]
struct EsploraTx {
    txid: String,
    #[serde(default)]
    vin: Vec<EsploraIn>,
    #[serde(default)]
    vout: Vec<EsploraOut>,
    status: EsploraStatus,
}

async fn esplora_history(asset: &str, bases: &[&str], address: &str) -> Result<Vec<Entry>> {

    let mut body: Vec<EsploraTx> = Vec::new();
    let mut last = WalletError::Network("no provider configured".into());
    let mut answered = false;
    for base in bases {
        match rpc::get_json::<Vec<EsploraTx>>(&format!("{base}{address}/txs")).await {
            Ok(v) => {
                body = v;
                answered = true;
                break;
            }
            Err(e) => last = e,
        }
    }
    if !answered {
        return Err(last);
    }

    Ok(body
        .into_iter()
        .take(LIMIT)
        .map(|tx| {

            let received: i128 = tx
                .vout
                .iter()
                .filter(|o| o.scriptpubkey_address.as_deref() == Some(address))
                .map(|o| o.value)
                .sum();
            let spent: i128 = tx
                .vin
                .iter()
                .filter_map(|i| i.prevout.as_ref())
                .filter(|o| o.scriptpubkey_address.as_deref() == Some(address))
                .map(|o| o.value)
                .sum();

            Entry::new(
                asset,
                tx.txid,
                received - spent,
                tx.status.block_time,
                tx.status.confirmed,
            )
        })
        .collect())
}

#[derive(Deserialize)]
struct SolSignature {
    signature: String,
    #[serde(rename = "blockTime")]
    block_time: Option<i64>,
    #[serde(default)]
    err: Option<serde_json::Value>,
    #[serde(rename = "confirmationStatus", default)]
    confirmation_status: Option<String>,
}

#[derive(Deserialize)]
struct SolKey {
    pubkey: String,
}

#[derive(Deserialize)]
struct SolMessage {
    #[serde(rename = "accountKeys", default)]
    account_keys: Vec<SolKey>,
}

#[derive(Deserialize)]
struct SolTransaction {
    message: SolMessage,
}

#[derive(Deserialize)]
struct SolMeta {
    #[serde(rename = "preBalances", default)]
    pre: Vec<i128>,
    #[serde(rename = "postBalances", default)]
    post: Vec<i128>,
}

#[derive(Deserialize)]
struct SolTxDetail {
    transaction: SolTransaction,
    meta: Option<SolMeta>,
}

async fn solana_history(address: &str) -> Result<Vec<Entry>> {
    let signatures: Vec<SolSignature> = rpc::sol_call(
        "getSignaturesForAddress",
        serde_json::json!([address, { "limit": LIMIT }]),
    )
    .await?;

    let details = futures::future::join_all(signatures.iter().map(|s| {
        let sig = s.signature.clone();
        async move {
            rpc::sol_call::<Option<SolTxDetail>>(
                "getTransaction",
                serde_json::json!([
                    sig,
                    { "encoding": "jsonParsed", "maxSupportedTransactionVersion": 0 }
                ]),
            )
            .await
        }
    }))
    .await;

    let mut out = Vec::new();
    for (summary, detail) in signatures.into_iter().zip(details) {

        if summary.err.is_some() {
            continue;
        }

        let net = match detail {
            Ok(Some(d)) => {
                let index = d
                    .transaction
                    .message
                    .account_keys
                    .iter()
                    .position(|k| k.pubkey == address);
                match (index, d.meta) {
                    (Some(i), Some(meta)) => {
                        let pre = meta.pre.get(i).copied().unwrap_or(0);
                        let post = meta.post.get(i).copied().unwrap_or(0);
                        post - pre
                    }
                    _ => continue,
                }
            }
            _ => continue,
        };

        if net == 0 {
            continue;
        }

        out.push(Entry::new(
            "SOL",
            summary.signature,
            net,
            summary.block_time,
            summary.confirmation_status.as_deref() == Some("finalized"),
        ));
    }

    Ok(out)
}

#[derive(Deserialize)]
struct TronValue {
    #[serde(default)]
    amount: Option<i128>,
    #[serde(default)]
    owner_address: Option<String>,
    #[serde(default)]
    to_address: Option<String>,
}

#[derive(Deserialize)]
struct TronParameter {
    #[serde(default)]
    value: Option<TronValue>,
}

#[derive(Deserialize)]
struct TronContract {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    parameter: Option<TronParameter>,
}

#[derive(Deserialize)]
struct TronRaw {
    #[serde(default)]
    contract: Vec<TronContract>,
}

#[derive(Deserialize)]
struct TronTx {
    #[serde(rename = "txID")]
    tx_id: String,
    #[serde(default)]
    block_timestamp: Option<i64>,
    #[serde(default)]
    raw_data: Option<TronRaw>,
}

#[derive(Deserialize)]
struct TronHistory {
    #[serde(default)]
    data: Vec<TronTx>,
}

fn tron_hex(address: &str) -> Option<String> {
    let raw: Vec<u8> = bs58::decode(address).with_check(None).into_vec().ok()?;
    Some(raw.iter().map(|b| format!("{b:02x}")).collect())
}

async fn tron_history(address: &str) -> Result<Vec<Entry>> {
    let ours = tron_hex(address)
        .ok_or_else(|| WalletError::Network("could not decode the Tron address".into()))?;

    let body: TronHistory = rpc::get_json(&format!(
        "https://api.trongrid.io/v1/accounts/{address}/transactions?limit={LIMIT}"
    ))
    .await?;

    Ok(body
        .data
        .into_iter()
        .filter_map(|tx| {
            let contract = tx.raw_data?.contract.into_iter().next()?;

            if contract.kind != "TransferContract" {
                return None;
            }
            let value = contract.parameter?.value?;
            let amount = value.amount?;

            let to = value.to_address.unwrap_or_default().to_lowercase();
            let from = value.owner_address.unwrap_or_default().to_lowercase();

            let net = if to == ours {
                amount
            } else if from == ours {
                -amount
            } else {
                return None;
            };

            Some(Entry::new(
                "TRON",
                tx.tx_id,
                net,
                tx.block_timestamp.map(|ms| ms / 1000),
                true,
            ))
        })
        .collect())
}

fn address_of(list: &[AssetAddress], asset: &str) -> Option<String> {
    list.iter()
        .find(|a| a.asset == asset)
        .and_then(|a| a.address.clone())
}

pub async fn utxo_history(asset: &str, address: &str) -> Result<Vec<Entry>> {
    match asset {
        "BTC" => {
            esplora_history(
                "BTC",
                &["https://mempool.space/api/address/", "https://blockstream.info/api/address/"],
                address,
            )
            .await
        }
        _ => esplora_history("LTC", &["https://litecoinspace.org/api/address/"], address).await,
    }
}

pub fn merge(entries: Vec<Entry>) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for e in entries {
        let net = |x: &Entry| {
            let v: i128 = x.amount_minor.parse().unwrap_or(0);
            if x.direction == "in" { v } else { -v }
        };
        match out.iter_mut().find(|x| x.asset == e.asset && x.id == e.id) {
            Some(x) => {
                let sum = net(x) + net(&e);
                *x = Entry::new(&e.asset, e.id.clone(), sum, x.timestamp.or(e.timestamp), x.confirmed && e.confirmed);
            }
            None => out.push(e),
        }
    }
    out.retain(|x| x.amount_minor != "0");
    out.sort_by(|x, y| match (y.timestamp, x.timestamp) {
        (Some(a), Some(b)) => a.cmp(&b),
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });
    out
}

pub async fn all(addresses: &[AssetAddress]) -> Vec<Entry> {
    let btc = address_of(addresses, "BTC");
    let ltc = address_of(addresses, "LTC");
    let sol = address_of(addresses, "SOL");
    let tron = address_of(addresses, "TRON");

    let (a, b, c, d) = tokio::join!(
        async {
            match &btc {
                Some(x) => {
                    esplora_history(
                        "BTC",
                        &[
                            "https://mempool.space/api/address/",
                            "https://blockstream.info/api/address/",
                        ],
                        x,
                    )
                    .await
                }
                None => Ok(vec![]),
            }
        },
        async {
            match &ltc {
                Some(x) => {
                    esplora_history("LTC", &["https://litecoinspace.org/api/address/"], x).await
                }
                None => Ok(vec![]),
            }
        },
        async {
            match &sol {
                Some(x) => solana_history(x).await,
                None => Ok(vec![]),
            }
        },
        async {
            match &tron {
                Some(x) => tron_history(x).await,
                None => Ok(vec![]),
            }
        },
    );

    let mut out: Vec<Entry> = [a, b, c, d]
        .into_iter()
        .filter_map(|r| r.ok())
        .flatten()
        .collect();

    out.sort_by(|x, y| match (y.timestamp, x.timestamp) {
        (Some(a), Some(b)) => a.cmp(&b),
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });

    out
}
