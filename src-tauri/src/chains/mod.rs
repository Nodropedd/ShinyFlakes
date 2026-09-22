//! Address derivation per chain.

pub mod btc_tx;
pub mod eth;
pub mod history;
pub mod rpc;
pub mod slip10;
pub mod sol_tx;
pub mod swap;
pub mod tokens;
pub mod tron_tx;
pub mod xmr;
pub mod xmr_rpc;
pub mod xmr_setup;

use bech32::{segwit, Hrp};
use bip32::{DerivationPath, XPrv};
use ripemd::Ripemd160;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sha3::Keccak256;

use crate::error::{Result, WalletError};

fn fail(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("{what}: {e}"))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetAddress {
    pub asset: String,
    pub address: Option<String>,

    pub path: Option<String>,

    pub host: Option<String>,

    pub unsupported: Option<String>,
}

fn secp_pubkey(seed: &[u8], path: &str) -> Result<k256::ecdsa::VerifyingKey> {
    let parsed: DerivationPath = path.parse().map_err(|e| fail("path", e))?;
    let xprv = XPrv::derive_from_path(seed, &parsed).map_err(|e| fail("derive", e))?;
    Ok(*xprv.public_key().public_key())
}

fn p2wpkh(seed: &[u8], path: &str, hrp: &str) -> Result<String> {
    let pubkey = secp_pubkey(seed, path)?;
    let compressed = pubkey.to_encoded_point(true);
    let hash160 = Ripemd160::digest(Sha256::digest(compressed.as_bytes()));
    let hrp = Hrp::parse(hrp).map_err(|e| fail("hrp", e))?;
    segwit::encode_v0(hrp, &hash160).map_err(|e| fail("bech32", e))
}

fn tron(seed: &[u8], path: &str) -> Result<String> {
    let pubkey = secp_pubkey(seed, path)?;
    let point = pubkey.to_encoded_point(false);

    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    let mut body = Vec::with_capacity(21);
    body.push(0x41);
    body.extend_from_slice(&hash[12..]);
    Ok(bs58::encode(body).with_check().into_string())
}

fn solana(seed: &[u8], path: &[u32]) -> Result<String> {
    let node = slip10::derive(seed, path);
    let signing = ed25519_dalek::SigningKey::from_bytes(&node.key);
    Ok(bs58::encode(signing.verifying_key().to_bytes()).into_string())
}

pub const BTC_PATH: &str = "m/84'/0'/0'/0/0";
pub const LTC_PATH: &str = "m/84'/2'/0'/0/0";
pub const TRON_PATH: &str = "m/44'/195'/0'/0/0";
pub const SOL_PATH: &[u32] = &[44, 501, 0, 0];
pub const SOL_PATH_TEXT: &str = "m/44'/501'/0'/0'";

pub fn addresses(seed: &[u8]) -> Result<Vec<AssetAddress>> {
    let btc = p2wpkh(seed, BTC_PATH, "bc")?;
    let ltc = p2wpkh(seed, LTC_PATH, "ltc")?;
    let trx = tron(seed, TRON_PATH)?;
    let sol = solana(seed, SOL_PATH)?;
    let xmr = xmr::address(seed)?;
    let eth_keys = eth::keys(seed)?;
    let eth_address = eth::to_checksum(&eth_keys.address);

    let owned = |asset: &str, address: &str, path: &str| AssetAddress {
        asset: asset.into(),
        address: Some(address.into()),
        path: Some(path.into()),
        host: None,
        unsupported: None,
    };

    let token = |asset: &str, address: &str, path: &str, host: &str| AssetAddress {
        asset: asset.into(),
        address: Some(address.into()),
        path: Some(path.into()),
        host: Some(host.into()),
        unsupported: None,
    };

    Ok(vec![
        owned("BTC", &btc, BTC_PATH),
        owned("LTC", &ltc, LTC_PATH),
        owned("XMR", &xmr, xmr::XMR_PATH),
        owned("ETH", &eth_address, eth::ETH_PATH),
        owned("SOL", &sol, SOL_PATH_TEXT),
        owned("TRON", &trx, TRON_PATH),

        token("USDC", &sol, SOL_PATH_TEXT, "SOL"),
        token("USDT", &trx, TRON_PATH, "TRON"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::seed;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon \
                          abandon abandon abandon abandon abandon about";

    fn test_seed() -> [u8; 64] {
        *seed::to_seed(&seed::parse(PHRASE).unwrap())
    }

    #[test]
    fn matches_the_bip84_bitcoin_vector() {

        assert_eq!(
            p2wpkh(&test_seed(), BTC_PATH, "bc").unwrap(),
            "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
        );
    }

    #[test]
    fn litecoin_uses_its_own_prefix_and_coin_type() {
        let ltc = p2wpkh(&test_seed(), LTC_PATH, "ltc").unwrap();
        assert!(ltc.starts_with("ltc1q"), "unexpected address: {ltc}");

        assert_ne!(ltc[4..], p2wpkh(&test_seed(), BTC_PATH, "bc").unwrap()[3..]);
    }

    #[test]
    fn tron_addresses_are_base58check_starting_with_t() {
        let addr = tron(&test_seed(), TRON_PATH).unwrap();
        assert!(addr.starts_with('T'), "unexpected address: {addr}");
        assert_eq!(addr.len(), 34);

        let decoded: Vec<u8> = bs58::decode(&addr).with_check(None).into_vec().unwrap();
        assert_eq!(decoded.len(), 21);
        assert_eq!(decoded[0], 0x41);
    }

    #[test]
    fn solana_addresses_are_32_byte_base58_keys() {
        let addr = solana(&test_seed(), SOL_PATH).unwrap();
        let decoded = bs58::decode(&addr).into_vec().unwrap();
        assert_eq!(decoded.len(), 32);
    }

    #[test]
    fn derivation_is_deterministic() {
        let a = addresses(&test_seed()).unwrap();
        let b = addresses(&test_seed()).unwrap();
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.address, y.address);
        }
    }

    #[test]
    fn a_different_seed_gives_different_addresses() {
        let other = *seed::to_seed(&seed::generate().and_then(|p| seed::parse(&p)).unwrap());
        let a = addresses(&test_seed()).unwrap();
        let b = addresses(&other).unwrap();
        for (x, y) in a.iter().zip(b.iter()) {
            if x.address.is_some() {
                assert_ne!(x.address, y.address, "{} collided", x.asset);
            }
        }
    }

    #[test]
    fn tokens_reuse_their_host_chain_address() {
        let all = addresses(&test_seed()).unwrap();
        let find = |id: &str| all.iter().find(|a| a.asset == id).unwrap().clone();
        assert!(find("ETH").address.unwrap().starts_with("0x"));
        assert_eq!(find("USDC").address, find("SOL").address);
        assert_eq!(find("USDT").address, find("TRON").address);
        assert!(find("XMR").address.unwrap().starts_with('4'));
    }
}

#[cfg(test)]
mod golden {
    use super::*;
    use crate::crypto::seed;

    const PHRASE: &str = "abandon abandon abandon abandon abandon abandon                           abandon abandon abandon abandon abandon about";

    #[test]
    fn reference_seed_addresses_do_not_drift() {
        let s = *seed::to_seed(&seed::parse(PHRASE).unwrap());
        let all = addresses(&s).unwrap();
        let get = |id: &str| {
            all.iter()
                .find(|a| a.asset == id)
                .unwrap()
                .address
                .clone()
        };

        assert_eq!(
            get("BTC").unwrap(),
            "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu"
        );
        assert_eq!(
            get("LTC").unwrap(),
            "ltc1qjmxnz78nmc8nq77wuxh25n2es7rzm5c2rkk4wh"
        );
        assert_eq!(
            get("SOL").unwrap(),
            "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"
        );
        assert_eq!(
            get("TRON").unwrap(),
            "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH"
        );
        assert_eq!(
            get("XMR").unwrap(),
            "43SMrTtLZsyZL81653f6b3BWpU5u6XZ2SRdAaM1MxLCGDcTq6mKi9D11ZgN2hbmCd             S9j66xu8Wz3J9wgiwkYssLnEK44756"
                .replace(' ', "")
        );
    }
}
