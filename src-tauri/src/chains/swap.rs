//! Swaps through ChangeNOW.

use serde_json::Value;

use crate::error::{Result, WalletError};

const HOST: &str = "https://api.changenow.io/v2";

fn net(e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(format!("swap: {e}"))
}

pub fn pair(asset: &str) -> Option<(&'static str, &'static str)> {
    Some(match asset {
        "BTC" => ("btc", "btc"),
        "LTC" => ("ltc", "ltc"),
        "XMR" => ("xmr", "xmr"),
        "ETH" => ("eth", "eth"),
        "SOL" => ("sol", "sol"),
        "TRON" => ("trx", "trx"),

        "USDC" => ("usdc", "sol"),
        "USDT" => ("usdt", "trx"),
        _ => return None,
    })
}

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

fn api_error(text: &str, status: reqwest::StatusCode) -> WalletError {
    let msg = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
            v.get("message")
                .or_else(|| v.get("error"))
                .and_then(|e| e.as_str().map(str::to_string))
        })
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("ChangeNOW returned {status}"));
    net(msg)
}

async fn get(path: &str, params: &[(&str, String)], api_key: &str) -> Result<Value> {
    let client = crate::chains::rpc::client()?;

    let url = reqwest::Url::parse_with_params(&format!("{HOST}{path}"), params)
        .map_err(net)?;
    let resp = client
        .get(url)
        .header("x-changenow-api-key", api_key)
        .send()
        .await
        .map_err(net)?;

    let status = resp.status();
    let text = resp.text().await.map_err(net)?;
    if !status.is_success() {
        return Err(api_error(&text, status));
    }
    serde_json::from_str(&text).map_err(|e| net(format!("unreadable response: {e}")))
}

async fn post(path: &str, body: Value, api_key: &str) -> Result<Value> {
    let client = crate::chains::rpc::client()?;
    let resp = client
        .post(format!("{HOST}{path}"))
        .header("x-changenow-api-key", api_key)
        .json(&body)
        .send()
        .await
        .map_err(net)?;

    let status = resp.status();
    let text = resp.text().await.map_err(net)?;
    if !status.is_success() {
        return Err(api_error(&text, status));
    }
    serde_json::from_str(&text).map_err(|e| net(format!("unreadable response: {e}")))
}

#[derive(Debug, Clone)]
pub struct Quote {
    pub amount_to: String,
    pub provider: String,
}

pub async fn quote(api_key: &str, from: &str, to: &str, amount_from: &str) -> Result<Quote> {
    let (tf, nf) = pair(from).ok_or_else(|| net(format!("{from} cannot be swapped")))?;
    let (tt, nt) = pair(to).ok_or_else(|| net(format!("{to} cannot be swapped")))?;

    let params = vec![
        ("fromCurrency", tf.to_string()),
        ("fromNetwork", nf.to_string()),
        ("toCurrency", tt.to_string()),
        ("toNetwork", nt.to_string()),
        ("fromAmount", amount_from.to_string()),
        ("flow", "standard".to_string()),
        ("type", "direct".to_string()),
    ];

    let v = get("/exchange/estimated-amount", &params, api_key).await?;
    let amount_to = str_field(&v, "toAmount");
    if amount_to.is_empty() || amount_to == "0" {
        return Err(net("no rate was returned for this pair and amount"));
    }

    Ok(Quote {
        amount_to,
        provider: "ChangeNOW".to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub id: String,
    pub deposit_address: String,
    pub deposit_memo: String,
    pub amount_from: String,
    pub amount_to: String,
    pub payout_address: String,
    pub provider: String,
    pub status: String,
}

pub async fn create(
    api_key: &str,
    from: &str,
    to: &str,
    amount_from: &str,
    payout: &str,
    refund: &str,
) -> Result<Trade> {
    let (tf, nf) = pair(from).ok_or_else(|| net(format!("{from} cannot be swapped")))?;
    let (tt, nt) = pair(to).ok_or_else(|| net(format!("{to} cannot be swapped")))?;

    let body = serde_json::json!({
        "fromCurrency": tf,
        "fromNetwork": nf,
        "toCurrency": tt,
        "toNetwork": nt,
        "fromAmount": amount_from,
        "toAmount": "",
        "address": payout,
        "extraId": "",
        "refundAddress": refund,
        "refundExtraId": "",
        "flow": "standard",
        "type": "direct",
        "userId": "",
        "payload": "",
        "contactEmail": "",
    });

    let v = post("/exchange", body, api_key).await?;

    let id = str_field(&v, "id");
    let deposit_address = str_field(&v, "payinAddress");
    if id.is_empty() || deposit_address.is_empty() {
        return Err(net("the trade could not be created"));
    }

    Ok(Trade {
        id,
        deposit_address,
        deposit_memo: str_field(&v, "payinExtraId"),
        amount_from: str_field(&v, "fromAmount"),
        amount_to: str_field(&v, "toAmount"),
        payout_address: str_field(&v, "payoutAddress"),
        provider: "ChangeNOW".to_string(),

        status: {
            let s = str_field(&v, "status");
            if s.is_empty() { "waiting".to_string() } else { s }
        },
    })
}

pub async fn status(api_key: &str, id: &str) -> Result<String> {
    let v = get("/exchange/by-id", &[("id", id.to_string())], api_key).await?;
    let s = str_field(&v, "status");
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

        assert_eq!(minor_to_decimal("100000000", 8).unwrap(), "1");

        assert_eq!(minor_to_decimal("1500000000", 9).unwrap(), "1.5");

        assert_eq!(minor_to_decimal("1000000", 9).unwrap(), "0.001");

        assert_eq!(minor_to_decimal("1", 8).unwrap(), "0.00000001");

        assert_eq!(minor_to_decimal("0", 8).unwrap(), "0");

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
        assert_eq!(pair("BTC"), Some(("btc", "btc")));
        assert_eq!(pair("USDT"), Some(("usdt", "trx")));
        assert_eq!(pair("USDC"), Some(("usdc", "sol")));
        assert_eq!(pair("DOGE"), None);
        assert_eq!(decimals("ETH"), Some(18));
        assert_eq!(decimals("DOGE"), None);
    }
}
