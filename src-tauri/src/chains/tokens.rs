//! Stablecoin token definitions.

pub const TOKEN_DECIMALS: u32 = 6;

pub fn is_token(asset: &str) -> bool {
    matches!(asset, "USDC" | "USDT")
}

pub fn contract(asset: &str, network: &str) -> Option<&'static str> {
    Some(match (asset, network) {

        ("USDC", "SOL") => "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        ("USDC", "ETH") => "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        ("USDC", "TRON") => "TEkxiTehnzSmSe2XqrBj4w32RUN966rdz8",

        ("USDT", "SOL") => "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        ("USDT", "ETH") => "0xdac17f958d2ee523a2206206994597c13d831ec7",
        ("USDT", "TRON") => "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_network_pair_has_a_contract() {
        for asset in ["USDC", "USDT"] {
            assert!(is_token(asset));
            for net in ["SOL", "ETH", "TRON"] {
                assert!(contract(asset, net).is_some(), "{asset} on {net}");
            }
        }
    }

    #[test]
    fn natives_are_not_tokens() {
        for asset in ["BTC", "LTC", "XMR", "ETH", "SOL", "TRON"] {
            assert!(!is_token(asset));
        }
    }

    #[test]
    fn unknown_pairings_are_none() {
        assert!(contract("USDC", "BTC").is_none());
        assert!(contract("BTC", "SOL").is_none());
    }
}
