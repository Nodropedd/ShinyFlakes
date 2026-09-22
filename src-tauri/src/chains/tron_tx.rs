//! Tron transaction building.

use bip32::{DerivationPath, XPrv};
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use sha2::{Digest, Sha256};
use sha3::Keccak256;

use crate::error::{Result, WalletError};

pub const TRON_PATH: &str = "m/44'/195'/0'/0/0";

pub const FEE_RESERVE_SUN: u64 = 1_100_000;

fn bad(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("tron {what}: {e}"))
}

pub fn signing_key(seed: &[u8]) -> Result<SigningKey> {
    let path: DerivationPath = TRON_PATH.parse().map_err(|e| bad("path", e))?;
    let xprv = XPrv::derive_from_path(seed, &path).map_err(|e| bad("derive", e))?;
    Ok(xprv.private_key().clone())
}

pub fn account_bytes(seed: &[u8]) -> Result<[u8; 21]> {
    let key = signing_key(seed)?;
    let point = key.verifying_key().to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    let mut body = [0u8; 21];
    body[0] = 0x41;
    body[1..].copy_from_slice(&hash[12..]);
    Ok(body)
}

pub fn address(seed: &[u8]) -> Result<String> {
    Ok(bs58::encode(account_bytes(seed)?).with_check().into_string())
}

pub fn parse_address(text: &str) -> Result<[u8; 21]> {
    let raw = bs58::decode(text.trim())
        .with_check(None)
        .into_vec()
        .map_err(|_| bad("address", "not a valid Tron address"))?;
    if raw.len() != 21 || raw[0] != 0x41 {
        return Err(bad("address", "not a mainnet Tron address"));
    }
    let mut out = [0u8; 21];
    out.copy_from_slice(&raw);
    Ok(out)
}

pub fn body20(account: &[u8; 21]) -> [u8; 20] {
    let mut out = [0u8; 20];
    out.copy_from_slice(&account[1..]);
    out
}

fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if v == 0 {
            break;
        }
    }
}

pub fn transfer_value(owner: &[u8; 21], to: &[u8; 21], amount: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    out.push(0x0a);
    varint(owner.len() as u64, &mut out);
    out.extend_from_slice(owner);
    out.push(0x12);
    varint(to.len() as u64, &mut out);
    out.extend_from_slice(to);
    out.push(0x18);
    varint(amount, &mut out);
    out
}

pub fn trc20_parameter(to: &[u8; 20], amount: u128) -> Vec<u8> {
    let mut out = vec![0u8; 64];
    out[12..32].copy_from_slice(to);
    out[48..64].copy_from_slice(&amount.to_be_bytes());
    out
}

pub fn txid(raw_data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(raw_data));
    out
}

pub fn sign_txid(key: &SigningKey, id: &[u8; 32]) -> Result<[u8; 65]> {
    let (sig, recovery): (Signature, RecoveryId) = key
        .sign_prehash_recoverable(id)
        .map_err(|e| bad("signing", e))?;
    let r = sig.r().to_bytes();
    let s = sig.s().to_bytes();

    let mut out = [0u8; 65];
    out[..32].copy_from_slice(r.as_slice());
    out[32..64].copy_from_slice(s.as_slice());
    out[64] = recovery.to_byte();
    Ok(out)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return Err(bad("hex", "odd length"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| bad("hex", e)))
        .collect()
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
    fn address_matches_the_derivation_module() {

        assert_eq!(address(&test_seed()).unwrap(), "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH");
    }

    #[test]
    fn account_bytes_carry_the_tag() {
        let a = account_bytes(&test_seed()).unwrap();
        assert_eq!(a[0], 0x41);
        assert_eq!(&body20(&a)[..], &a[1..]);
    }

    #[test]
    fn parses_a_valid_address_and_rejects_junk() {
        let owner = address(&test_seed()).unwrap();
        assert_eq!(parse_address(&owner).unwrap(), account_bytes(&test_seed()).unwrap());
        assert!(parse_address("not-an-address").is_err());

        assert!(parse_address("bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu").is_err());
    }

    #[test]
    fn transfer_value_is_the_expected_protobuf() {
        let owner = [0x41u8; 21];
        let to = {
            let mut t = [0u8; 21];
            t[0] = 0x41;
            t
        };
        let v = transfer_value(&owner, &to, 1_000_000);

        assert_eq!(v[0], 0x0a);
        assert_eq!(v[1], 21);
        assert_eq!(&v[2..23], &owner);

        assert_eq!(v[23], 0x12);
        assert_eq!(v[24], 21);
        assert_eq!(&v[25..46], &to);

        assert_eq!(v[46], 0x18);
        let mut expect = Vec::new();
        varint(1_000_000, &mut expect);
        assert_eq!(&v[47..], expect.as_slice());
    }

    #[test]
    fn trc20_parameter_right_aligns_both_words() {
        let to = [0xabu8; 20];
        let p = trc20_parameter(&to, 5_000_000);
        assert_eq!(p.len(), 64);
        assert_eq!(&p[..12], &[0u8; 12]);
        assert_eq!(&p[12..32], &to);
        assert_eq!(u128::from_be_bytes(p[48..64].try_into().unwrap()), 5_000_000);
    }

    #[test]
    fn varint_matches_known_values() {
        let mut out = Vec::new();
        varint(0, &mut out);
        assert_eq!(out, vec![0x00]);
        out.clear();
        varint(300, &mut out);
        assert_eq!(out, vec![0xac, 0x02]);
    }

    #[test]
    fn signing_is_deterministic_and_65_bytes() {
        let key = signing_key(&test_seed()).unwrap();
        let id = txid(b"a tron raw data body");
        let a = sign_txid(&key, &id).unwrap();
        let b = sign_txid(&key, &id).unwrap();
        assert_eq!(a, b, "RFC 6979 signing is deterministic");
        assert!(a[64] <= 1, "recovery id is 0 or 1");
    }

    #[test]
    fn hex_round_trips() {
        let bytes = [0x00, 0x41, 0xff, 0xab];
        assert_eq!(unhex(&hex(&bytes)).unwrap(), bytes);
    }
}
