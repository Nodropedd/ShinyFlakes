//! Ethereum keys, addresses and transactions.
//!
//! Sending uses the EIP-1559 transaction type, which is what the network has
//! priced in since London and what every current wallet emits. The older
//! gas-price form still works but overpays.
//!
//! Amounts are held as u128 rather than u64 throughout: wei has eighteen
//! decimal places, so a balance above about 18.4 ETH would overflow a u64.

use bip32::{DerivationPath, XPrv};
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use sha3::{Digest, Keccak256};

use crate::error::{Result, WalletError};

pub const ETH_PATH: &str = "m/44'/60'/0'/0/0";
pub const CHAIN_ID: u64 = 1;

/// A plain transfer costs exactly this much gas. Contract calls cost more and
/// are estimated instead.
pub const TRANSFER_GAS: u64 = 21_000;

fn bad(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("ethereum {what}: {e}"))
}

fn keccak(data: &[u8]) -> [u8; 32] {
    let out = Keccak256::digest(data);
    let mut result = [0u8; 32];
    result.copy_from_slice(&out);
    result
}

// ---------- keys and addresses ----------

pub struct Keys {
    pub signing: SigningKey,
    pub address: [u8; 20],
}

pub fn keys(seed: &[u8]) -> Result<Keys> {
    let path: DerivationPath = ETH_PATH.parse().map_err(|e| bad("path", e))?;
    let xprv = XPrv::derive_from_path(seed, &path).map_err(|e| bad("derive", e))?;
    let signing = xprv.private_key().clone();

    let point = signing.verifying_key().to_encoded_point(false);
    // Drop the 0x04 tag; the address is the hash of the raw coordinates.
    let hash = keccak(&point.as_bytes()[1..]);

    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    Ok(Keys { signing, address })
}

/// Mixed-case checksum form from EIP-55. The capitalisation is not cosmetic:
/// it is a checksum, and wallets use it to catch mistyped addresses.
pub fn to_checksum(address: &[u8; 20]) -> String {
    let lower: String = address.iter().map(|b| format!("{b:02x}")).collect();
    let hash = keccak(lower.as_bytes());

    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, c) in lower.chars().enumerate() {
        // Each hex character is checked against its own nibble of the hash.
        let nibble = if i % 2 == 0 {
            hash[i / 2] >> 4
        } else {
            hash[i / 2] & 0x0f
        };
        if c.is_ascii_digit() || nibble < 8 {
            out.push(c);
        } else {
            out.push(c.to_ascii_uppercase());
        }
    }
    out
}

/// Parses a destination address, rejecting a bad EIP-55 checksum.
///
/// An all-lowercase or all-uppercase address carries no checksum, so it is
/// accepted as-is. A mixed-case one that fails the check is a typo, and
/// paying it would send the money nowhere recoverable.
pub fn parse_address(text: &str) -> Result<[u8; 20]> {
    let trimmed = text.trim();
    let hex = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    if hex.len() != 40 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad("address", "expected 40 hexadecimal characters after 0x"));
    }

    let mut address = [0u8; 20];
    for i in 0..20 {
        address[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|e| bad("address", e))?;
    }

    let mixed = hex.chars().any(|c| c.is_ascii_uppercase())
        && hex.chars().any(|c| c.is_ascii_lowercase());
    if mixed && to_checksum(&address)[2..] != *hex {
        return Err(bad(
            "address",
            "the checksum does not match, so this address has a typo in it",
        ));
    }

    Ok(address)
}

// ---------- RLP ----------

/// Recursive length prefix, the encoding Ethereum uses for transactions.
fn rlp_string(data: &[u8], out: &mut Vec<u8>) {
    if data.len() == 1 && data[0] < 0x80 {
        out.push(data[0]);
    } else if data.len() <= 55 {
        out.push(0x80 + data.len() as u8);
        out.extend_from_slice(data);
    } else {
        let len = data.len().to_be_bytes();
        let trimmed = &len[len.iter().position(|b| *b != 0).unwrap_or(len.len() - 1)..];
        out.push(0xb7 + trimmed.len() as u8);
        out.extend_from_slice(trimmed);
        out.extend_from_slice(data);
    }
}

fn rlp_list(payload: &[u8], out: &mut Vec<u8>) {
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
    } else {
        let len = payload.len().to_be_bytes();
        let trimmed = &len[len.iter().position(|b| *b != 0).unwrap_or(len.len() - 1)..];
        out.push(0xf7 + trimmed.len() as u8);
        out.extend_from_slice(trimmed);
    }
    out.extend_from_slice(payload);
}

/// Numbers are encoded as the shortest big-endian byte string, and zero is
/// the empty string rather than a zero byte.
fn rlp_uint(value: u128, out: &mut Vec<u8>) {
    let bytes = value.to_be_bytes();
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(bytes.len());
    rlp_string(&bytes[start..], out);
}

// ---------- transactions ----------

pub struct Transfer {
    pub nonce: u64,
    pub max_priority_fee: u128,
    pub max_fee: u128,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
}

fn payload(tx: &Transfer) -> Vec<u8> {
    let mut fields = Vec::new();
    rlp_uint(CHAIN_ID as u128, &mut fields);
    rlp_uint(tx.nonce as u128, &mut fields);
    rlp_uint(tx.max_priority_fee, &mut fields);
    rlp_uint(tx.max_fee, &mut fields);
    rlp_uint(tx.gas_limit as u128, &mut fields);
    rlp_string(&tx.to, &mut fields);
    rlp_uint(tx.value, &mut fields);
    rlp_string(&tx.data, &mut fields);
    // Empty access list.
    fields.push(0xc0);
    fields
}

/// The digest signed for an EIP-1559 transaction: the type byte followed by
/// the RLP-encoded fields, hashed with keccak.
pub fn signing_hash(tx: &Transfer) -> [u8; 32] {
    let mut encoded = vec![0x02];
    rlp_list(&payload(tx), &mut encoded);
    keccak(&encoded)
}

/// Signs and returns the bytes to submit via eth_sendRawTransaction.
pub fn sign(keys: &Keys, tx: &Transfer) -> Result<Vec<u8>> {
    let digest = signing_hash(tx);

    let (signature, recovery): (Signature, RecoveryId) = keys
        .signing
        .sign_prehash_recoverable(&digest)
        .map_err(|e| bad("signing", e))?;

    // EIP-1559 carries the parity bit directly rather than folding the chain
    // id into it the way pre-EIP-155 transactions did.
    let r = signature.r().to_bytes();
    let s = signature.s().to_bytes();

    let mut fields = payload(tx);
    rlp_uint(recovery.to_byte() as u128, &mut fields);
    rlp_string(r.as_slice(), &mut fields);
    rlp_string(s.as_slice(), &mut fields);

    let mut out = vec![0x02];
    rlp_list(&fields, &mut out);
    Ok(out)
}

/// Transaction id, which is the keccak hash of the submitted bytes.
///
/// The node returns this too, so it is only needed if one answers oddly.
#[allow(dead_code)]
pub fn tx_hash(signed: &[u8]) -> String {
    format!(
        "0x{}",
        keccak(signed)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

// ---------- ERC-20 ----------

/// Call data for `transfer(address,uint256)`.
///
/// Written and tested ahead of the USDT-on-Ethereum work, which needs it.
#[allow(dead_code)]
///
/// The selector is the first four bytes of the keccak hash of the signature,
/// followed by both arguments padded to 32 bytes each.
pub fn erc20_transfer_data(to: &[u8; 20], amount: u128) -> Vec<u8> {
    let selector = &keccak(b"transfer(address,uint256)")[..4];

    let mut data = Vec::with_capacity(68);
    data.extend_from_slice(selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(to);
    data.extend_from_slice(&[0u8; 16]);
    data.extend_from_slice(&amount.to_be_bytes());
    data
}

/// Call data for `balanceOf(address)`.
#[allow(dead_code)]
pub fn erc20_balance_data(owner: &[u8; 20]) -> Vec<u8> {
    let selector = &keccak(b"balanceOf(address)")[..4];
    let mut data = Vec::with_capacity(36);
    data.extend_from_slice(selector);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(owner);
    data
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

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The address every Ethereum tool produces for the reference mnemonic at
    /// the standard path. Widely published, and the check that the whole
    /// derivation chain is right.
    #[test]
    fn matches_the_reference_address() {
        let k = keys(&test_seed()).unwrap();
        assert_eq!(
            to_checksum(&k.address),
            "0x9858EfFD232B4033E47d90003D41EC34EcaEda94"
        );
    }

    /// The four examples given in EIP-55 itself.
    #[test]
    fn checksums_match_eip55() {
        let cases = [
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
            "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
            "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        ];
        for expected in cases {
            let parsed = parse_address(expected).unwrap();
            assert_eq!(to_checksum(&parsed), expected);
        }
    }

    #[test]
    fn rejects_a_mistyped_address() {
        // One character case flipped: a real typo would look like this.
        let wrong = "0x5aAeb6053f3E94C9b9A09f33669435E7Ef1BeAed";
        assert!(parse_address(wrong).is_err());
    }

    #[test]
    fn accepts_addresses_without_a_checksum() {
        // All lowercase carries no checksum, so it cannot be wrong.
        assert!(parse_address("0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed").is_ok());
    }

    #[test]
    fn rejects_malformed_addresses() {
        assert!(parse_address("0x1234").is_err());
        assert!(parse_address("").is_err());
        assert!(parse_address("0xZZZZb6053f3e94c9b9a09f33669435e7ef1beaed").is_err());
    }

    #[test]
    fn rlp_encodes_the_documented_examples() {
        let mut out = Vec::new();
        rlp_string(b"dog", &mut out);
        assert_eq!(hex(&out), "83646f67");

        // A single byte below 0x80 is its own encoding.
        out.clear();
        rlp_string(&[0x0f], &mut out);
        assert_eq!(hex(&out), "0f");

        // The empty string.
        out.clear();
        rlp_string(&[], &mut out);
        assert_eq!(hex(&out), "80");

        // Zero encodes as the empty string, not as a zero byte.
        out.clear();
        rlp_uint(0, &mut out);
        assert_eq!(hex(&out), "80");

        out.clear();
        rlp_uint(1024, &mut out);
        assert_eq!(hex(&out), "820400");

        // A list of two short strings.
        let mut payload = Vec::new();
        rlp_string(b"cat", &mut payload);
        rlp_string(b"dog", &mut payload);
        out.clear();
        rlp_list(&payload, &mut out);
        assert_eq!(hex(&out), "c88363617483646f67");
    }

    #[test]
    fn rlp_handles_long_strings() {
        let long = vec![b'a'; 56];
        let mut out = Vec::new();
        rlp_string(&long, &mut out);
        // 0xb8 then one length byte then the payload.
        assert_eq!(out[0], 0xb8);
        assert_eq!(out[1], 56);
        assert_eq!(out.len(), 58);
    }

    #[test]
    fn erc20_call_data_is_the_right_shape() {
        let to = parse_address("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").unwrap();
        let data = erc20_transfer_data(&to, 1_000_000);

        assert_eq!(data.len(), 4 + 32 + 32);
        // The published selector for transfer(address,uint256).
        assert_eq!(hex(&data[..4]), "a9059cbb");
        // Address is right-aligned in its 32 byte word.
        assert_eq!(&data[4..16], &[0u8; 12]);
        assert_eq!(&data[16..36], &to);
        // Amount is right-aligned in its own 32 byte word, so the low 16
        // bytes hold it and the high 16 are padding.
        assert_eq!(&data[36..52], &[0u8; 16]);
        assert_eq!(u128::from_be_bytes(data[52..68].try_into().unwrap()), 1_000_000);
    }

    #[test]
    fn signing_produces_a_typed_transaction() {
        let k = keys(&test_seed()).unwrap();
        let tx = Transfer {
            nonce: 0,
            max_priority_fee: 1_000_000_000,
            max_fee: 30_000_000_000,
            gas_limit: TRANSFER_GAS,
            to: parse_address("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").unwrap(),
            value: 1_000_000_000_000_000,
            data: Vec::new(),
        };

        let signed = sign(&k, &tx).unwrap();
        assert_eq!(signed[0], 0x02, "EIP-1559 transactions carry a type byte");
        assert!(signed.len() > 90);
        assert!(tx_hash(&signed).starts_with("0x"));
        assert_eq!(tx_hash(&signed).len(), 66);

        // Deterministic signing: the same transaction signs identically.
        assert_eq!(hex(&sign(&k, &tx).unwrap()), hex(&signed));
    }
}
