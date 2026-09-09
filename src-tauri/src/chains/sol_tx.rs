//! Solana transfer construction and signing.
//!
//! Written against the legacy transaction format rather than pulling in
//! solana-sdk, which drags a very large dependency tree behind it. The wire
//! format is small and stable, so it is spelled out here where it can be read
//! and tested.
//!
//! Layout of a signed transaction:
//!
//!   compact-array of 64-byte signatures
//!   message:
//!     header          3 bytes
//!     account keys    compact-array of 32-byte public keys
//!     recent blockhash 32 bytes
//!     instructions    compact-array of { program index, account indices, data }
//!
//! Lengths use Solana's compact-u16: seven bits per byte, high bit set while
//! more bytes follow.

use ed25519_dalek::{Signer, SigningKey};

use super::slip10;
use crate::error::{Result, WalletError};

/// The System program, whose id is thirty-two zero bytes.
const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];

/// Index of Transfer within the System program's instruction enum.
const TRANSFER_INSTRUCTION: u32 = 2;

/// Solana charges per signature. One signer here, so one unit.
pub const LAMPORTS_PER_SIGNATURE: u64 = 5_000;

fn bad(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("solana {what}: {e}"))
}

/// Solana's compact-u16 length prefix.
fn push_compact_u16(out: &mut Vec<u8>, mut value: u16) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

/// Decodes a base58 account address into its 32 byte public key.
pub fn parse_address(address: &str) -> Result<[u8; 32]> {
    let raw = bs58::decode(address.trim())
        .into_vec()
        .map_err(|e| bad("address", e))?;
    if raw.len() != 32 {
        return Err(bad(
            "address",
            format!("expected 32 bytes, got {}", raw.len()),
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&raw);
    Ok(key)
}

/// Serialises the message that actually gets signed.
///
/// Account ordering is fixed by the format: writable signers first, then
/// writable non-signers, then read-only. Here that is sender, recipient,
/// system program.
pub fn build_message(from: &[u8; 32], to: &[u8; 32], lamports: u64, blockhash: &[u8; 32]) -> Vec<u8> {
    let mut msg = Vec::new();

    // Header: one required signature, no read-only signers, one read-only
    // unsigned account (the System program).
    msg.push(1);
    msg.push(0);
    msg.push(1);

    push_compact_u16(&mut msg, 3);
    msg.extend_from_slice(from);
    msg.extend_from_slice(to);
    msg.extend_from_slice(&SYSTEM_PROGRAM);

    msg.extend_from_slice(blockhash);

    // One instruction.
    push_compact_u16(&mut msg, 1);
    msg.push(2); // program is account index 2
    push_compact_u16(&mut msg, 2);
    msg.push(0); // from
    msg.push(1); // to

    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&TRANSFER_INSTRUCTION.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    push_compact_u16(&mut msg, data.len() as u16);
    msg.extend_from_slice(&data);

    msg
}

/// The account's ed25519 signing key, derived at the Solana path.
pub fn signing_key(seed: &[u8]) -> SigningKey {
    let node = slip10::derive(seed, super::SOL_PATH);
    SigningKey::from_bytes(&node.key)
}

/// Builds and signs a transfer, returning the bytes to submit.
pub fn signed_transfer(
    seed: &[u8],
    to: &str,
    lamports: u64,
    blockhash: &[u8; 32],
) -> Result<Vec<u8>> {
    let key = signing_key(seed);
    let from = key.verifying_key().to_bytes();
    let to = parse_address(to)?;

    if from == to {
        return Err(bad("recipient", "sending to your own address"));
    }

    let message = build_message(&from, &to, lamports, blockhash);
    let signature = key.sign(&message);

    let mut tx = Vec::with_capacity(1 + 64 + message.len());
    push_compact_u16(&mut tx, 1);
    tx.extend_from_slice(&signature.to_bytes());
    tx.extend_from_slice(&message);
    Ok(tx)
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
    fn compact_u16_matches_the_spec() {
        let mut out = Vec::new();
        push_compact_u16(&mut out, 0);
        assert_eq!(out, vec![0]);

        out.clear();
        push_compact_u16(&mut out, 127);
        assert_eq!(out, vec![127]);

        // 128 needs a continuation byte.
        out.clear();
        push_compact_u16(&mut out, 128);
        assert_eq!(out, vec![0x80, 0x01]);

        out.clear();
        push_compact_u16(&mut out, 16384);
        assert_eq!(out, vec![0x80, 0x80, 0x01]);
    }

    #[test]
    fn rejects_addresses_of_the_wrong_size() {
        assert!(parse_address("not base58 at all!!").is_err());
        // Valid base58, wrong length.
        assert!(parse_address("abc").is_err());
    }

    #[test]
    fn accepts_a_real_address() {
        let key = parse_address("HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk").unwrap();
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn refuses_to_send_to_itself() {
        let seed = test_seed();
        let own = bs58::encode(signing_key(&seed).verifying_key().to_bytes()).into_string();
        assert!(signed_transfer(&seed, &own, 1, &[7u8; 32]).is_err());
    }

    #[test]
    fn message_layout_is_exact() {
        let from = [1u8; 32];
        let to = [2u8; 32];
        let blockhash = [3u8; 32];
        let msg = build_message(&from, &to, 1_000_000, &blockhash);

        // 3 header + 1 count + 96 keys + 32 blockhash + 1 count
        // + 1 program + 1 count + 2 indices + 1 len + 12 data
        assert_eq!(msg.len(), 150);
        assert_eq!(&msg[0..3], &[1, 0, 1]);
        assert_eq!(msg[3], 3, "three accounts");
        assert_eq!(&msg[4..36], &from);
        assert_eq!(&msg[36..68], &to);
        assert_eq!(&msg[68..100], &SYSTEM_PROGRAM);
        assert_eq!(&msg[100..132], &blockhash);

        // Instruction data: transfer discriminant then the amount.
        let data = &msg[msg.len() - 12..];
        assert_eq!(&data[0..4], &TRANSFER_INSTRUCTION.to_le_bytes());
        assert_eq!(&data[4..12], &1_000_000u64.to_le_bytes());
    }

    #[test]
    fn signature_verifies_against_the_message() {
        use ed25519_dalek::Verifier;

        let seed = test_seed();
        let tx = signed_transfer(
            &seed,
            "11111111111111111111111111111112",
            1_000,
            &[9u8; 32],
        )
        .unwrap();

        assert_eq!(tx[0], 1, "one signature");
        let signature = ed25519_dalek::Signature::from_bytes(
            &tx[1..65].try_into().expect("64 byte signature"),
        );
        let message = &tx[65..];

        let key = signing_key(&seed).verifying_key();
        assert!(key.verify(message, &signature).is_ok());
    }
}
