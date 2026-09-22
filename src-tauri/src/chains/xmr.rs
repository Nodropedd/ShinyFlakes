//! Monero keys and balance.

use bip32::{DerivationPath, XPrv};
use curve25519_dalek::scalar::Scalar;
use monero::{Address, KeyPair, Network, PrivateKey};
use sha3::{Digest, Keccak256};
use zeroize::Zeroizing;

use crate::error::{Result, WalletError};

pub const XMR_PATH: &str = "m/44'/128'/0'/0/0";

fn fail(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("monero {what}: {e}"))
}

pub struct Keys {
    pub spend: PrivateKey,
    pub view: PrivateKey,
}

impl Drop for Keys {
    fn drop(&mut self) {

        self.spend = PrivateKey::from_scalar(Scalar::ZERO);
        self.view = PrivateKey::from_scalar(Scalar::ZERO);
    }
}

pub fn keys(seed: &[u8]) -> Result<Keys> {
    let path: DerivationPath = XMR_PATH.parse().map_err(|e| fail("path", e))?;
    let xprv = XPrv::derive_from_path(seed, &path).map_err(|e| fail("derive", e))?;

    let raw = Zeroizing::new(xprv.private_key().to_bytes());
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(raw.as_slice());

    let spend_scalar = Scalar::from_bytes_mod_order(bytes);
    let spend = PrivateKey::from_scalar(spend_scalar);

    let view_scalar = Scalar::from_bytes_mod_order(Keccak256::digest(spend.to_bytes()).into());
    let view = PrivateKey::from_scalar(view_scalar);

    Ok(Keys { spend, view })
}

pub fn address(seed: &[u8]) -> Result<String> {
    let keys = keys(seed)?;
    let pair = KeyPair {
        view: keys.view,
        spend: keys.spend,
    };
    Ok(Address::from_keypair(Network::Mainnet, &pair).to_string())
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
    fn produces_a_standard_mainnet_address() {
        let addr = address(&test_seed()).unwrap();

        assert!(addr.starts_with('4'), "unexpected address: {addr}");
        assert_eq!(addr.len(), 95, "unexpected length: {addr}");
    }

    #[test]
    fn the_address_parses_back() {
        let addr = address(&test_seed()).unwrap();
        let parsed: Address = addr.parse().expect("monero should accept its own address");
        assert_eq!(parsed.network, Network::Mainnet);
        assert_eq!(parsed.to_string(), addr);
    }

    #[test]
    fn view_key_follows_the_spend_key() {

        let k = keys(&test_seed()).unwrap();
        let expected = Scalar::from_bytes_mod_order(Keccak256::digest(k.spend.to_bytes()).into());
        assert_eq!(k.view.to_bytes(), PrivateKey::from_scalar(expected).to_bytes());
    }

    #[test]
    fn is_deterministic_and_seed_specific() {
        assert_eq!(address(&test_seed()).unwrap(), address(&test_seed()).unwrap());
        let other = *seed::to_seed(&seed::generate().and_then(|p| seed::parse(&p)).unwrap());
        assert_ne!(address(&test_seed()).unwrap(), address(&other).unwrap());
    }
}
