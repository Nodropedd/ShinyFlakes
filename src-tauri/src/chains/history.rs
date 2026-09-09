//! Transaction history.
//!
//! Each chain reports its past differently, so every one is normalised here
//! into the same shape: what moved, which way, when, and whether it settled.
//!
//! Amounts are the net effect on this wallet, not the raw transaction value.
//! A Bitcoin transaction spending a large input and returning most of it as
//! change moved only the difference, and that is what belongs in a history.

use serde::{Deserialize, Serialize};

use super::rpc;
use super::AssetAddress;
use crate::error::{Result, WalletError};

/// How many entries to pull from each chain.
const LIMIT: usize = 15;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub asset: String,
    pub id: String,
    /// "in" or "out", from this wallet's point of view.
    pub direction: String,
    /// Absolute net movement in the asset's smallest unit.
    pub amount_minor: String,
    /// Unix seconds. Absent while a transaction is still unconfirmed.
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

// ---------- Bitcoin and Litecoin ----------

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
    // Same reasoning as the balance lookups: one provider is not enough.
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
            // Outputs paying us, minus our own inputs being spent. Change
            // returning to the same address cancels itself out, which is what
            // makes this the amount that actually left.
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

// ---------- Solana ----------

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

    // Signatures alone carry no amount, so each one has to be fetched. They
    // are independent, so they go out together rather than in sequence.
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
        // A failed transaction still cost a fee, but reporting it as a
        // transfer would be wrong.
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

// ---------- Tron ----------

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

/// Tron reports addresses as hex with the same 0x41 prefix the base58 form
/// encodes, so ours is converted once for comparison.
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
            // Only plain transfers carry an amount that belongs in a balance
            // history. Contract calls and staking are a different story.
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

// ---------- Entry point ----------

fn address_of(list: &[AssetAddress], asset: &str) -> Option<String> {
    list.iter()
        .find(|a| a.asset == asset)
        .and_then(|a| a.address.clone())
}

/// Everything the wallet can see, newest first.
///
/// Monero is absent for the same reason its balance is: history there means
/// scanning the chain with the view key. Token transfers on Solana and Tron
/// are not included yet either; only the native coin of each chain is.
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

    // A chain that will not answer should cost its own entries, not everyone
    // else's, so failures are dropped rather than propagated.
    let mut out: Vec<Entry> = [a, b, c, d]
        .into_iter()
        .filter_map(|r| r.ok())
        .flatten()
        .collect();

    // Unconfirmed entries have no time yet and belong at the top.
    out.sort_by(|x, y| match (y.timestamp, x.timestamp) {
        (Some(a), Some(b)) => a.cmp(&b),
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });

    out
}
