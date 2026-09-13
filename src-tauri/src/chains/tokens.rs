//! Which contract or mint carries each stablecoin on each network, and the
//! shared facts about the tokens this wallet moves.
//!
//! USDC and USDT are the same unit of account wherever they live, but each
//! network holds a different token: an SPL mint on Solana, an ERC-20 contract
//! on Ethereum, a TRC-20 contract on Tron. Sending one means picking the
//! network, then using that chain's own transfer. Addresses are shared with
//! the native coin of the host chain — USDC on Solana is received at your
//! Solana address, and so on.
//!
//! Every address below is well known and was cross-checked against public
//! sources; still, a wrong one would send funds nowhere, so verify with a
//! small amount before trusting a route with real money.

/// The networks a stablecoin can live on, in display order.
pub const NETWORKS: [&str; 3] = ["SOL", "ETH", "TRON"];

/// Both stablecoins use six decimals on every network here.
pub const TOKEN_DECIMALS: u32 = 6;

/// True for the assets that are tokens rather than native coins.
pub fn is_token(asset: &str) -> bool {
    matches!(asset, "USDC" | "USDT")
}

/// The host chain's native asset id for a token network, e.g. the Solana
/// account that holds an SPL token. Used to find the address and signer.
pub fn host_asset(network: &str) -> Option<&'static str> {
    Some(match network {
        "SOL" => "SOL",
        "ETH" => "ETH",
        "TRON" => "TRON",
        _ => return None,
    })
}

/// The networks a given token can be sent on. Empty for a native coin.
pub fn networks_for(asset: &str) -> &'static [&'static str] {
    if is_token(asset) {
        &NETWORKS
    } else {
        &[]
    }
}

/// The mint (Solana) or contract (Ethereum, Tron) address for a token on a
/// network. `None` when that pairing is not one this wallet carries.
pub fn contract(asset: &str, network: &str) -> Option<&'static str> {
    Some(match (asset, network) {
        // Circle's USDC.
        ("USDC", "SOL") => "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        ("USDC", "ETH") => "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
        ("USDC", "TRON") => "TEkxiTehnzSmSe2XqrBj4w32RUN966rdz8",
        // Tether's USDT.
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
            for net in networks_for(asset) {
                assert!(contract(asset, net).is_some(), "{asset} on {net}");
                assert!(host_asset(net).is_some());
            }
        }
    }

    #[test]
    fn natives_are_not_tokens() {
        for asset in ["BTC", "LTC", "XMR", "ETH", "SOL", "TRON"] {
            assert!(!is_token(asset));
            assert!(networks_for(asset).is_empty());
        }
    }

    #[test]
    fn unknown_pairings_are_none() {
        assert!(contract("USDC", "BTC").is_none());
        assert!(contract("BTC", "SOL").is_none());
    }
}
