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

use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use super::slip10;
use crate::error::{Result, WalletError};

/// The System program, whose id is thirty-two zero bytes.
const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];

/// Index of Transfer within the System program's instruction enum.
const TRANSFER_INSTRUCTION: u32 = 2;

/// The SPL Token program and the Associated Token Account program.
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
/// The literal appended when hashing a program-derived address.
const PDA_MARKER: &[u8] = b"ProgramDerivedAddress";
/// TransferChecked within the token program's instruction enum. Checked rather
/// than plain Transfer so the mint and decimals are verified on chain.
const TRANSFER_CHECKED: u8 = 12;
/// CreateIdempotent within the ATA program: makes the recipient's token
/// account if it is missing, and is a no-op if it already exists.
const CREATE_ATA_IDEMPOTENT: u8 = 1;

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
    build_message_multi(from, &[(*to, lamports)], blockhash)
}

/// A transfer message paying one or more recipients from the same account.
///
/// One recipient is an ordinary send; two carry the creator fee alongside it.
/// Account order is fixed: the signer, then each recipient, then the System
/// program last. Each recipient gets its own transfer instruction.
pub fn build_message_multi(
    from: &[u8; 32],
    dests: &[([u8; 32], u64)],
    blockhash: &[u8; 32],
) -> Vec<u8> {
    let mut msg = Vec::new();

    let num_accounts = 1 + dests.len() + 1; // signer + recipients + system
    let system_index = (num_accounts - 1) as u8;

    // Header: one required signature, no read-only signers, one read-only
    // unsigned account (the System program).
    msg.push(1);
    msg.push(0);
    msg.push(1);

    push_compact_u16(&mut msg, num_accounts as u16);
    msg.extend_from_slice(from);
    for (to, _) in dests {
        msg.extend_from_slice(to);
    }
    msg.extend_from_slice(&SYSTEM_PROGRAM);

    msg.extend_from_slice(blockhash);

    push_compact_u16(&mut msg, dests.len() as u16);
    for (i, (_, lamports)) in dests.iter().enumerate() {
        msg.push(system_index); // program is the System account
        push_compact_u16(&mut msg, 2);
        msg.push(0); // from, the signer
        msg.push((1 + i) as u8); // this recipient

        let mut data = Vec::with_capacity(12);
        data.extend_from_slice(&TRANSFER_INSTRUCTION.to_le_bytes());
        data.extend_from_slice(&lamports.to_le_bytes());
        push_compact_u16(&mut msg, data.len() as u16);
        msg.extend_from_slice(&data);
    }

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
    Ok(sign_message(&key, &message))
}

/// A transfer that also pays the creator fee, in one transaction.
///
/// When the fee is zero this is just the plain single transfer, so the caller
/// need not special-case it.
pub fn signed_transfer_with_fee(
    seed: &[u8],
    to: &str,
    lamports: u64,
    fee_to: &str,
    fee_lamports: u64,
    blockhash: &[u8; 32],
) -> Result<Vec<u8>> {
    let key = signing_key(seed);
    let from = key.verifying_key().to_bytes();
    let to = parse_address(to)?;
    if from == to {
        return Err(bad("recipient", "sending to your own address"));
    }

    let mut dests = vec![(to, lamports)];
    if fee_lamports > 0 {
        let fee_to = parse_address(fee_to)?;
        // Paying the fee address as the recipient is caught earlier, but guard
        // against a degenerate two-output-to-one-account message anyway.
        if fee_to != to {
            dests.push((fee_to, fee_lamports));
        }
    }

    let message = build_message_multi(&from, &dests, blockhash);
    Ok(sign_message(&key, &message))
}

fn sign_message(key: &SigningKey, message: &[u8]) -> Vec<u8> {
    let signature = key.sign(message);
    let mut tx = Vec::with_capacity(1 + 64 + message.len());
    push_compact_u16(&mut tx, 1);
    tx.extend_from_slice(&signature.to_bytes());
    tx.extend_from_slice(message);
    tx
}

// ---------- SPL tokens ----------

/// True when 32 bytes are a valid ed25519 point. A program-derived address is
/// specifically one that is NOT, which is how it proves no private key exists
/// for it.
fn on_curve(bytes: &[u8; 32]) -> bool {
    CompressedEdwardsY(*bytes).decompress().is_some()
}

/// Solana's `create_program_address`: hash the seeds, the program id and the
/// marker; the result is a PDA only if it lands off the curve.
fn create_program_address(seeds: &[&[u8]], program: &[u8; 32]) -> Option<[u8; 32]> {
    let mut h = Sha256::new();
    for s in seeds {
        h.update(s);
    }
    h.update(program);
    h.update(PDA_MARKER);
    let out: [u8; 32] = h.finalize().into();
    if on_curve(&out) {
        None
    } else {
        Some(out)
    }
}

/// `find_program_address`: the highest bump seed that yields an off-curve
/// address. Every account program uses this to place its accounts.
fn find_program_address(seeds: &[&[u8]], program: &[u8; 32]) -> [u8; 32] {
    for bump in (0u8..=255).rev() {
        let tail = [bump];
        let mut all: Vec<&[u8]> = seeds.to_vec();
        all.push(&tail);
        if let Some(addr) = create_program_address(&all, program) {
            return addr;
        }
    }
    // Finding no off-curve bump is cryptographically impossible in practice.
    [0u8; 32]
}

/// The associated token account that holds `mint` for `owner`: the deterministic
/// address every wallet uses, derived from the owner, the token program and the
/// mint under the ATA program.
pub fn associated_token_account(owner: &[u8; 32], mint: &[u8; 32]) -> [u8; 32] {
    let token = parse_address(TOKEN_PROGRAM).expect("token program id is valid");
    let ata = parse_address(ATA_PROGRAM).expect("ata program id is valid");
    find_program_address(&[owner, &token, mint], &ata)
}

/// Builds the message for an SPL transfer. Two instructions: create the
/// recipient's token account if it is missing (idempotent, so harmless if it
/// exists), then a checked transfer into it. The signer pays the fee and any
/// account rent.
pub fn build_spl_message(
    owner: &[u8; 32],
    recipient: &[u8; 32],
    mint: &[u8; 32],
    amount: u64,
    decimals: u8,
    blockhash: &[u8; 32],
) -> Vec<u8> {
    let token = parse_address(TOKEN_PROGRAM).expect("token program id is valid");
    let ata_prog = parse_address(ATA_PROGRAM).expect("ata program id is valid");
    let source_ata = associated_token_account(owner, mint);
    let dest_ata = associated_token_account(recipient, mint);

    // Fixed order: writable signer, writable non-signers, then read-only.
    let accounts: [[u8; 32]; 8] = [
        *owner,         // 0  signer, writable (fee payer + token owner)
        source_ata,     // 1  writable
        dest_ata,       // 2  writable
        *recipient,     // 3  read-only (the recipient's wallet)
        *mint,          // 4  read-only
        SYSTEM_PROGRAM, // 5  read-only
        token,          // 6  read-only (token program)
        ata_prog,       // 7  read-only (ATA program)
    ];

    let mut msg = Vec::new();
    // One required signature; five read-only unsigned accounts (indices 3..=7).
    msg.push(1);
    msg.push(0);
    msg.push(5);

    push_compact_u16(&mut msg, accounts.len() as u16);
    for a in &accounts {
        msg.extend_from_slice(a);
    }

    msg.extend_from_slice(blockhash);

    push_compact_u16(&mut msg, 2);

    // 1) Create the recipient's associated token account if needed.
    msg.push(7); // program: ATA
    push_compact_u16(&mut msg, 6);
    for i in [0u8, 2, 3, 4, 5, 6] {
        msg.push(i);
    }
    push_compact_u16(&mut msg, 1);
    msg.push(CREATE_ATA_IDEMPOTENT);

    // 2) TransferChecked from our account to theirs.
    msg.push(6); // program: token
    push_compact_u16(&mut msg, 4);
    for i in [1u8, 4, 2, 0] {
        msg.push(i);
    }
    let mut data = Vec::with_capacity(10);
    data.push(TRANSFER_CHECKED);
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    push_compact_u16(&mut msg, data.len() as u16);
    msg.extend_from_slice(&data);

    msg
}

/// Builds and signs an SPL token transfer to `to`'s wallet address.
pub fn signed_spl_transfer(
    seed: &[u8],
    to: &str,
    mint: &str,
    amount: u64,
    decimals: u8,
    blockhash: &[u8; 32],
) -> Result<Vec<u8>> {
    let key = signing_key(seed);
    let owner = key.verifying_key().to_bytes();
    let recipient = parse_address(to)?;
    let mint = parse_address(mint)?;
    if recipient == owner {
        return Err(bad("recipient", "sending to your own address"));
    }
    let message = build_spl_message(&owner, &recipient, &mint, amount, decimals, blockhash);
    Ok(sign_message(&key, &message))
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
    fn a_two_recipient_message_has_four_accounts_and_two_instructions() {
        let from = [1u8; 32];
        let to = [2u8; 32];
        let fee = [3u8; 32];
        let blockhash = [4u8; 32];
        let msg = build_message_multi(&from, &[(to, 900), (fee, 100)], &blockhash);

        // Header, then four accounts: signer, two recipients, System last.
        assert_eq!(&msg[0..3], &[1, 0, 1]);
        assert_eq!(msg[3], 4);
        assert_eq!(&msg[4..36], &from);
        assert_eq!(&msg[36..68], &to);
        assert_eq!(&msg[68..100], &fee);
        assert_eq!(&msg[100..132], &SYSTEM_PROGRAM);
        // Blockhash, then a count of two instructions.
        assert_eq!(&msg[132..164], &blockhash);
        assert_eq!(msg[164], 2);
    }

    #[test]
    fn one_recipient_still_matches_the_original_layout() {
        // The single-recipient wrapper must be byte-identical to the layout
        // the BIP-tested path relied on: three accounts, one instruction.
        let from = [7u8; 32];
        let to = [8u8; 32];
        let blockhash = [9u8; 32];
        let a = build_message(&from, &to, 5, &blockhash);
        let b = build_message_multi(&from, &[(to, 5)], &blockhash);
        assert_eq!(a, b);
        assert_eq!(a[3], 3, "three accounts");
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
    fn real_keys_are_on_curve_and_atas_are_not() {
        // A real public key decompresses to a curve point; a program-derived
        // account is chosen precisely because it does not. This checks the
        // curve test both ways, which is the heart of ATA derivation.
        let owner = signing_key(&test_seed()).verifying_key().to_bytes();
        assert!(on_curve(&owner), "a real public key is on the curve");
        let mint = parse_address("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        assert!(on_curve(&mint), "a mint address is a real key, on the curve");
        let ata = associated_token_account(&owner, &mint);
        assert!(!on_curve(&ata), "a derived token account is off the curve");
    }

    #[test]
    fn ata_is_deterministic_and_mint_specific() {
        let owner = signing_key(&test_seed()).verifying_key().to_bytes();
        let usdc = parse_address("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        let usdt = parse_address("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").unwrap();
        assert_eq!(
            associated_token_account(&owner, &usdc),
            associated_token_account(&owner, &usdc),
        );
        assert_ne!(
            associated_token_account(&owner, &usdc),
            associated_token_account(&owner, &usdt),
        );
    }

    #[test]
    fn spl_message_has_eight_accounts_and_two_instructions() {
        let owner = [1u8; 32];
        let recipient = [2u8; 32];
        let mint = [3u8; 32];
        let blockhash = [4u8; 32];
        let msg = build_spl_message(&owner, &recipient, &mint, 1_000_000, 6, &blockhash);

        // Header: one signature, no read-only signers, five read-only unsigned.
        assert_eq!(&msg[0..3], &[1, 0, 5]);
        assert_eq!(msg[3], 8, "eight accounts");
        assert_eq!(&msg[4..36], &owner, "the signer comes first");
        // 3 header + 1 count + 8*32 keys + 32 blockhash = 292: the instruction
        // count, which must be two (create-if-needed, then transfer).
        assert_eq!(msg[292], 2, "two instructions");
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
