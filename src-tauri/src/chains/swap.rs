//! Cross-chain swaps through Trocador, a non-custodial aggregator.
//!
//! No account of ours and no custody. The wallet asks Trocador for a rate and a
//! deposit address, pays a normal on-chain send to it from the "from" coin, and
//! the exchange Trocador picks sends the "to" coin straight to an address this
//! wallet owns. Keys never leave this machine; Trocador is just another remote
//! endpoint, like the balance and price lookups, and it rides the same Tor
//! routing when that is on.
//!
//! Monetisation is Trocador's own referral markup, set by whoever holds the API
//! key, not an extra output of ours. The creator fee that ordinary sends carry
//! is deliberately NOT applied to a swap's funding transaction: it would change
//! the exact deposit amount the exchange is waiting for and break the swap.
//!
//! The ticker and network strings below are Trocador's own, matching how the
//! reference privacy wallets (Cake, Feather) name the same coins. Amounts cross
//! to Trocador as decimal strings; we convert without floats so a satoshi or a
//! wei is never lost.

use serde_json::Value;

use crate::error::{Result, WalletError};

const HOST: &str = "https://api.trocador.app";

fn net(e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(format!("swap: {e}"))
}

/// Ticker and network as Trocador names them, per asset. `None` for an asset
/// Trocador is not asked about. If a pair is ever rejected as unknown, this is
/// the one table to correct.
pub fn pair(asset: &str) -> Option<(&'static str, &'static str)> {
    Some(match asset {
        "BTC" => ("btc", "Mainnet"),
        "LTC" => ("ltc", "Mainnet"),
        "XMR" => ("xmr", "Mainnet"),
        "ETH" => ("eth", "ERC20"),
        "SOL" => ("sol", "Mainnet"),
        "TRON" => ("trx", "Mainnet"),
        // Ours are the Solana USDC and the Tron USDT, so the networks are fixed.
        "USDC" => ("usdc", "SOL"),
        "USDT" => ("usdt", "TRC20"),
        _ => return None,
    })
}

/// Smallest-unit decimals per asset, for converting to and from the decimal
/// strings Trocador speaks.
pub fn decimals(asset: &str) -> Option<u32> {
    Some(match asset {
        "BTC" | "LTC" => 8,
        "XMR" => 12,
        "ETH" => 18,
        "SOL" => 9,
        "TRON" | "USDC" | "USDT" => 6,
        _ => return None,
    })
}

/// A smallest-unit integer string to a plain decimal string, exactly.
pub fn minor_to_decimal(minor: &str, dp: u32) -> Result<String> {
    let raw = minor.trim();
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WalletError::Derivation(format!("not a whole number: {minor}")));
    }
    let digits = raw.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let dp = dp as usize;
    if dp == 0 {
        return Ok(digits.to_string());
    }
    let (int, frac) = if digits.len() <= dp {
        ("0".to_string(), format!("{digits:0>dp$}"))
    } else {
        let split = digits.len() - dp;
        (digits[..split].to_string(), digits[split..].to_string())
    };
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        Ok(int)
    } else {
        Ok(format!("{int}.{frac}"))
    }
}

/// A plain decimal string back to a smallest-unit integer. Rejects more
/// precision than the asset has, rather than silently dropping it.
pub fn decimal_to_minor(dec: &str, dp: u32) -> Result<u128> {
    let s = dec.trim();
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(WalletError::Derivation(format!("not a number: {dec}")));
    }
    let dp = dp as usize;
    if frac.len() > dp {
        return Err(WalletError::Derivation(format!(
            "{dec} is finer than this asset can hold"
        )));
    }
    let mut combined = String::new();
    combined.push_str(if int.is_empty() { "0" } else { int });
    combined.push_str(frac);
    combined.extend(std::iter::repeat('0').take(dp - frac.len()));
    let trimmed = combined.trim_start_matches('0');
    if trimmed.is_empty() {
        return Ok(0);
    }
    trimmed
        .parse::<u128>()
        .map_err(|_| WalletError::Derivation(format!("amount out of range: {dec}")))
}

/// Like `decimal_to_minor`, but floors extra precision instead of refusing it.
/// For amounts coming back from Trocador, which the wallet only displays.
pub fn decimal_to_minor_floor(dec: &str, dp: u32) -> Result<u128> {
    let s = dec.trim();
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    let frac: String = frac.chars().take(dp as usize).collect();
    let clamped = if frac.is_empty() {
        int.to_string()
    } else {
        format!("{int}.{frac}")
    };
    decimal_to_minor(&clamped, dp)
}

fn str_field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// A GET against Trocador, returning the parsed JSON or a readable error.
async fn get(path: &str, params: &[(&str, String)], api_key: &str) -> Result<Value> {
    let client = crate::chains::rpc::client()?;
    // Built with the query encoder rather than `.query()`, which this reqwest
    // build does not expose, so amounts and addresses are escaped correctly.
    let url = reqwest::Url::parse_with_params(&format!("{HOST}{path}"), params)
        .map_err(net)?;
    let resp = client
        .get(url)
        .header("API-Key", api_key)
        .send()
        .await
        .map_err(net)?;

    let status = resp.status();
    let text = resp.text().await.map_err(net)?;

    if !status.is_success() {
        // Bad requests come back as {"error": "..."}; surface that, not a code.
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str().map(str::to_string)))
            .unwrap_or_else(|| format!("Trocador returned {status}"));
        return Err(net(msg));
    }

    serde_json::from_str(&text).map_err(|e| net(format!("unreadable response: {e}")))
}

/// A rate estimate. `amount_to` is what the "to" side is expected to receive; a
/// variable-rate swap can settle a little different, which the UI says.
#[derive(Debug, Clone)]
pub struct Quote {
    pub amount_to: String,
    pub provider: String,
}

fn markup_params(markup: f64) -> Vec<(&'static str, String)> {
    if markup > 0.0 && markup.is_finite() {
        vec![("markup", format!("{markup}"))]
    } else {
        Vec::new()
    }
}

/// A rate for sending `amount_from` (a decimal string) of one asset into another.
pub async fn quote(
    api_key: &str,
    markup: f64,
    from: &str,
    to: &str,
    amount_from: &str,
) -> Result<Quote> {
    let (tf, nf) = pair(from).ok_or_else(|| net(format!("{from} cannot be swapped")))?;
    let (tt, nt) = pair(to).ok_or_else(|| net(format!("{to} cannot be swapped")))?;

    let mut params = vec![
        ("ticker_from", tf.to_string()),
        ("network_from", nf.to_string()),
        ("ticker_to", tt.to_string()),
        ("network_to", nt.to_string()),
        ("amount_from", amount_from.to_string()),
        ("payment", "False".to_string()),
    ];
    params.extend(markup_params(markup));

    let v = get("/new_rate", &params, api_key).await?;
    let amount_to = str_field(&v, "amount_to");
    if amount_to.is_empty() {
        return Err(net("no rate was returned for this pair and amount"));
    }
    // The best provider, when the response names one.
    let provider = v
        .get("quotes")
        .and_then(|q| q.as_array())
        .and_then(|a| a.first())
        .map(|q| str_field(q, "provider"))
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| str_field(&v, "provider"));

    Ok(Quote { amount_to, provider })
}

/// A created trade: where to deposit, how much, and where the proceeds go.
#[derive(Debug, Clone)]
pub struct Trade {
    pub id: String,
    pub deposit_address: String,
    pub deposit_memo: String,
    pub amount_from: String,
    pub amount_to: String,
    pub payout_address: String,
    pub refund_address: String,
    pub provider: String,
    pub status: String,
}

/// Creates the trade. `payout` is an address this wallet owns on the "to"
/// chain; `refund` is one it owns on the "from" chain, used if the swap fails.
#[allow(clippy::too_many_arguments)]
pub async fn create(
    api_key: &str,
    markup: f64,
    from: &str,
    to: &str,
    amount_from: &str,
    payout: &str,
    refund: &str,
) -> Result<Trade> {
    let (tf, nf) = pair(from).ok_or_else(|| net(format!("{from} cannot be swapped")))?;
    let (tt, nt) = pair(to).ok_or_else(|| net(format!("{to} cannot be swapped")))?;

    let mut params = vec![
        ("ticker_from", tf.to_string()),
        ("network_from", nf.to_string()),
        ("ticker_to", tt.to_string()),
        ("network_to", nt.to_string()),
        ("amount_from", amount_from.to_string()),
        ("address", payout.to_string()),
        ("refund", refund.to_string()),
        ("payment", "False".to_string()),
    ];
    params.extend(markup_params(markup));

    let v = get("/new_trade", &params, api_key).await?;

    let id = str_field(&v, "trade_id");
    let deposit_address = str_field(&v, "address_provider");
    if id.is_empty() || deposit_address.is_empty() {
        return Err(net("the trade could not be created"));
    }

    Ok(Trade {
        id,
        deposit_address,
        deposit_memo: str_field(&v, "address_provider_memo"),
        amount_from: str_field(&v, "amount_from"),
        amount_to: str_field(&v, "amount_to"),
        payout_address: str_field(&v, "address_user"),
        refund_address: str_field(&v, "refund_address"),
        provider: str_field(&v, "provider"),
        status: str_field(&v, "status"),
    })
}

/// The current status of a trade by its id. Trocador returns an array; the
/// first entry is the trade.
pub async fn status(api_key: &str, id: &str) -> Result<String> {
    let v = get("/trade", &[("id", id.to_string())], api_key).await?;
    let entry = match &v {
        Value::Array(a) => a.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    let s = str_field(&entry, "status");
    if s.is_empty() {
        Err(net("the trade could not be found"))
    } else {
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minor_to_decimal_places_the_point_correctly() {
        // 1 BTC.
        assert_eq!(minor_to_decimal("100000000", 8).unwrap(), "1");
        // 1.5 SOL.
        assert_eq!(minor_to_decimal("1500000000", 9).unwrap(), "1.5");
        // 0.001 SOL, shorter than the decimal places.
        assert_eq!(minor_to_decimal("1000000", 9).unwrap(), "0.001");
        // One satoshi.
        assert_eq!(minor_to_decimal("1", 8).unwrap(), "0.00000001");
        // Zero.
        assert_eq!(minor_to_decimal("0", 8).unwrap(), "0");
        // A wei-scale value keeps every digit.
        assert_eq!(
            minor_to_decimal("1234567890123456789", 18).unwrap(),
            "1.234567890123456789"
        );
    }

    #[test]
    fn decimal_to_minor_is_the_inverse() {
        assert_eq!(decimal_to_minor("1", 8).unwrap(), 100_000_000);
        assert_eq!(decimal_to_minor("1.5", 9).unwrap(), 1_500_000_000);
        assert_eq!(decimal_to_minor("0.001", 9).unwrap(), 1_000_000);
        assert_eq!(decimal_to_minor("0.00000001", 8).unwrap(), 1);
        assert_eq!(decimal_to_minor("0", 8).unwrap(), 0);
        assert_eq!(decimal_to_minor(".5", 2).unwrap(), 50);
    }

    #[test]
    fn round_trips_hold() {
        for (minor, dp) in [("100000000", 8), ("1", 12), ("999999999999999999", 18)] {
            let dec = minor_to_decimal(minor, dp).unwrap();
            assert_eq!(decimal_to_minor(&dec, dp).unwrap().to_string(), minor);
        }
    }

    #[test]
    fn extra_precision_is_refused_not_truncated() {
        // 9 places into an 8-place asset would drop a satoshi silently.
        assert!(decimal_to_minor("0.000000001", 8).is_err());
    }

    #[test]
    fn junk_amounts_are_rejected() {
        assert!(minor_to_decimal("12x4", 8).is_err());
        assert!(minor_to_decimal("", 8).is_err());
        assert!(decimal_to_minor("1.2.3", 8).is_err());
        assert!(decimal_to_minor("abc", 8).is_err());
    }

    #[test]
    fn only_known_assets_map() {
        assert_eq!(pair("BTC"), Some(("btc", "Mainnet")));
        assert_eq!(pair("USDT"), Some(("usdt", "TRC20")));
        assert_eq!(pair("USDC"), Some(("usdc", "SOL")));
        assert_eq!(pair("DOGE"), None);
        assert_eq!(decimals("ETH"), Some(18));
        assert_eq!(decimals("DOGE"), None);
    }
}
