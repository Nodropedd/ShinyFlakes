//! Tron keys, addresses and transaction signing.
//!
//! Tron transactions are protobuf, and building one needs a recent block to
//! reference. Rather than reimplement the block lookup and the full protobuf
//! wire format, the node builds the transaction skeleton (`createtransaction`
//! for TRX, `triggersmartcontract` for TRC-20) and this module signs it. That
//! keeps signing local — the key never leaves the machine — while leaning on
//! the node for the parts that are pure plumbing.
//!
//! Trusting the node to build the skeleton would be a hole, though: a hostile
//! response could name a different recipient or amount. So before signing, the
//! caller rebuilds the security-critical part of the contract (owner, to,
//! amount) with `transfer_value` / `trc20_value` and checks the node's
//! `raw_data_hex` actually contains those exact bytes, and that the id it
//! returned really is the hash of that raw data. Only then is the id signed.

use bip32::{DerivationPath, XPrv};
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use sha2::{Digest, Sha256};
use sha3::Keccak256;

use crate::error::{Result, WalletError};

pub const TRON_PATH: &str = "m/44'/195'/0'/0/0";

/// A plain transfer with no free bandwidth burns about 0.27 TRX. This is the
/// headroom the send flow keeps back so a max-send is not rejected for the fee;
/// with bandwidth available the transfer is free and this is simply untouched.
pub const FEE_RESERVE_SUN: u64 = 1_100_000;

fn bad(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("tron {what}: {e}"))
}

/// The secp256k1 key that signs for this wallet's Tron account.
pub fn signing_key(seed: &[u8]) -> Result<SigningKey> {
    let path: DerivationPath = TRON_PATH.parse().map_err(|e| bad("path", e))?;
    let xprv = XPrv::derive_from_path(seed, &path).map_err(|e| bad("derive", e))?;
    Ok(xprv.private_key().clone())
}

/// The 21-byte account: the 0x41 tag followed by the keccak-derived 20 bytes,
/// the same body that base58check-encodes to a "T..." address.
pub fn account_bytes(seed: &[u8]) -> Result<[u8; 21]> {
    let key = signing_key(seed)?;
    let point = key.verifying_key().to_encoded_point(false);
    let hash = Keccak256::digest(&point.as_bytes()[1..]);
    let mut body = [0u8; 21];
    body[0] = 0x41;
    body[1..].copy_from_slice(&hash[12..]);
    Ok(body)
}

/// This wallet's Tron address, base58check.
pub fn address(seed: &[u8]) -> Result<String> {
    Ok(bs58::encode(account_bytes(seed)?).with_check().into_string())
}

/// Parses and checksums a destination "T..." address into its 21-byte form.
/// A bad checksum is a typo, and paying it would send the money nowhere.
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

/// The 20-byte body (no 0x41 tag), as a TRC-20 call expects an address.
pub fn body20(account: &[u8; 21]) -> [u8; 20] {
    let mut out = [0u8; 20];
    out.copy_from_slice(&account[1..]);
    out
}

// ---------- protobuf (only what verification needs) ----------

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

/// The protobuf encoding of a TransferContract's value: owner (field 1),
/// to (field 2), amount (field 3). These bytes appear verbatim inside the raw
/// data the node returns, so finding them there proves the node did not alter
/// the recipient or amount.
pub fn transfer_value(owner: &[u8; 21], to: &[u8; 21], amount: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    out.push(0x0a); // field 1, length-delimited
    varint(owner.len() as u64, &mut out);
    out.extend_from_slice(owner);
    out.push(0x12); // field 2, length-delimited
    varint(to.len() as u64, &mut out);
    out.extend_from_slice(to);
    out.push(0x18); // field 3, varint
    varint(amount, &mut out);
    out
}

/// The `parameter` bytes of a TRC-20 `transfer(address,uint256)` call: the
/// recipient right-aligned in 32 bytes, then the amount right-aligned in 32.
pub fn trc20_parameter(to: &[u8; 20], amount: u128) -> Vec<u8> {
    let mut out = vec![0u8; 64];
    out[12..32].copy_from_slice(to);
    out[48..64].copy_from_slice(&amount.to_be_bytes());
    out
}

// ---------- signing ----------

/// The transaction id: the SHA-256 of the raw-data bytes.
pub fn txid(raw_data: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(raw_data));
    out
}

/// Signs a 32-byte transaction id, returning Tron's 65-byte signature:
/// r, then s, then the one-byte recovery id.
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

/// Lowercase hex, for the signature and for substring checks against raw data.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes a hex string, for turning the node's `raw_data_hex` back into bytes.
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
        // Same reference address the derivation golden test locks in.
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
        // A bech32 address is not a Tron address.
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
        // field 1: 0x0a, len 21, 21 bytes.
        assert_eq!(v[0], 0x0a);
        assert_eq!(v[1], 21);
        assert_eq!(&v[2..23], &owner);
        // field 2: 0x12, len 21, 21 bytes.
        assert_eq!(v[23], 0x12);
        assert_eq!(v[24], 21);
        assert_eq!(&v[25..46], &to);
        // field 3: 0x18 then the amount as a varint (1_000_000).
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
