//! Where a tip goes.
//!
//! These are fixed, compiled in, and shown in full before anything is sent.
//! A wallet that could quietly change its own payout address would be a very
//! different kind of program.
//!
//! Every address here is checked by the same parser that validates a
//! recipient the user types, so a typo cannot ship.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Donation {
    pub asset: String,
    pub address: String,
    /// Set when the asset is a token and the address belongs to its host
    /// chain rather than to the token itself.
    pub host: Option<String>,
}

/// USD Coin lives on Solana in this wallet and Tether on Tron, so tips in
/// either go to the address for the chain that actually carries them.
pub fn addresses() -> Vec<Donation> {
    let sol = "7oW2bBM5iU4At2ZBJqv81zG7dV2XGWeBHdDYozdZojnb";
    let tron = "TKoYYY3jnZHUJhgS8HodUXKzwhZQjDyeLW";

    let owned = |asset: &str, address: &str| Donation {
        asset: asset.into(),
        address: address.into(),
        host: None,
    };
    let token = |asset: &str, address: &str, host: &str| Donation {
        asset: asset.into(),
        address: address.into(),
        host: Some(host.into()),
    };

    vec![
        owned("BTC", "bc1qpy3p4gxa4d3x3w0lryma77qqdrfgwhfqwhw7tn"),
        owned("LTC", "LVhh3tqqbo7bQmQtCgsbuVbdVvCpKhEpCy"),
        owned(
            "XMR",
            "42oUemzbsb9A5fWPhaCaBbKmafMXqyTpSKnE8Rco5iyjQNN7NYmct8CS7HFcA8omm6ABgBzDy2NPQTu1zubFH3UuRLwnNUL",
        ),
        owned("ETH", "0x7aE8380cF08BD44629d099F05eD85570a8d7B930"),
        owned("SOL", sol),
        owned("TRON", tron),
        token("USDC", sol, "SOL"),
        token("USDT", tron, "TRON"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::btc_tx::{script_pubkey_for, Chain};

    fn find(asset: &str) -> Donation {
        addresses()
            .into_iter()
            .find(|d| d.asset == asset)
            .unwrap_or_else(|| panic!("{asset} missing from the donation list"))
    }

    /// Each address is run through the parser that would validate it if a
    /// user typed it as a recipient. A wrong one fails here rather than
    /// silently sending a tip nowhere.
    #[test]
    fn bitcoin_address_is_spendable_to() {
        let script = script_pubkey_for(&find("BTC").address, Chain::Bitcoin).unwrap();
        // Native segwit: a zero byte, a length, then a 20 byte program.
        assert_eq!(script[0], 0x00);
        assert_eq!(script.len(), 22);
    }

    #[test]
    fn litecoin_address_is_spendable_to() {
        let script = script_pubkey_for(&find("LTC").address, Chain::Litecoin).unwrap();
        // Legacy pay-to-pubkey-hash on Litecoin.
        assert_eq!(script[0], 0x76);
        assert_eq!(script.len(), 25);
    }

    #[test]
    fn litecoin_address_is_not_a_bitcoin_one() {
        // Sending Litecoin to a Bitcoin address, or the reverse, loses it.
        assert!(script_pubkey_for(&find("LTC").address, Chain::Bitcoin).is_err());
        assert!(script_pubkey_for(&find("BTC").address, Chain::Litecoin).is_err());
    }

    #[test]
    fn ethereum_address_passes_its_checksum() {
        // Mixed case, so EIP-55 applies and a single wrong character fails.
        let parsed = crate::chains::eth::parse_address(&find("ETH").address).unwrap();
        assert_eq!(
            crate::chains::eth::to_checksum(&parsed),
            find("ETH").address
        );
    }

    #[test]
    fn solana_address_is_a_public_key() {
        let key = crate::chains::sol_tx::parse_address(&find("SOL").address).unwrap();
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn tron_address_passes_its_checksum() {
        let raw: Vec<u8> = bs58::decode(&find("TRON").address)
            .with_check(None)
            .into_vec()
            .expect("base58check");
        assert_eq!(raw.len(), 21);
        assert_eq!(raw[0], 0x41, "Tron addresses carry the 0x41 tag");
    }

    #[test]
    fn monero_address_parses_as_mainnet() {
        let parsed: monero::Address = find("XMR").address.parse().expect("monero address");
        assert_eq!(parsed.network, monero::Network::Mainnet);
    }

    #[test]
    fn tokens_point_at_their_host_chain() {
        assert_eq!(find("USDC").address, find("SOL").address);
        assert_eq!(find("USDC").host.as_deref(), Some("SOL"));
        assert_eq!(find("USDT").address, find("TRON").address);
        assert_eq!(find("USDT").host.as_deref(), Some("TRON"));
    }

    #[test]
    fn every_asset_the_wallet_holds_can_be_tipped() {
        for asset in ["BTC", "LTC", "XMR", "ETH", "SOL", "TRON", "USDC", "USDT"] {
            let _ = find(asset);
        }
    }
}
