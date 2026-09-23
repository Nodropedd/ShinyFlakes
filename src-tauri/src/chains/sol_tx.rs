//! Solana transaction building.

use curve25519_dalek::edwards::CompressedEdwardsY;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use crate::error::{Result, WalletError};

const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];

const TRANSFER_INSTRUCTION: u32 = 2;

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const ATA_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

const PDA_MARKER: &[u8] = b"ProgramDerivedAddress";

const TRANSFER_CHECKED: u8 = 12;

const CREATE_ATA_IDEMPOTENT: u8 = 1;

pub const LAMPORTS_PER_SIGNATURE: u64 = 5_000;

fn bad(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("solana {what}: {e}"))
}

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

pub fn build_message(from: &[u8; 32], to: &[u8; 32], lamports: u64, blockhash: &[u8; 32]) -> Vec<u8> {
    build_message_multi(from, &[(*to, lamports)], blockhash)
}

pub fn build_message_multi(
    from: &[u8; 32],
    dests: &[([u8; 32], u64)],
    blockhash: &[u8; 32],
) -> Vec<u8> {
    let mut msg = Vec::new();

    let num_accounts = 1 + dests.len() + 1;
    let system_index = (num_accounts - 1) as u8;

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
        msg.push(system_index);
        push_compact_u16(&mut msg, 2);
        msg.push(0);
        msg.push((1 + i) as u8);

        let mut data = Vec::with_capacity(12);
        data.extend_from_slice(&TRANSFER_INSTRUCTION.to_le_bytes());
        data.extend_from_slice(&lamports.to_le_bytes());
        push_compact_u16(&mut msg, data.len() as u16);
        msg.extend_from_slice(&data);
    }

    msg
}

pub fn signing_key(seed: &[u8]) -> Result<SigningKey> {
    super::sol_key(seed, super::sol_exodus())
}

pub fn signed_transfer(
    seed: &[u8],
    to: &str,
    lamports: u64,
    blockhash: &[u8; 32],
) -> Result<Vec<u8>> {
    let key = signing_key(seed)?;
    let from = key.verifying_key().to_bytes();
    let to = parse_address(to)?;

    if from == to {
        return Err(bad("recipient", "sending to your own address"));
    }

    let message = build_message(&from, &to, lamports, blockhash);
    Ok(sign_message(&key, &message))
}

pub fn signed_transfer_with_fee(
    seed: &[u8],
    to: &str,
    lamports: u64,
    fee_to: &str,
    fee_lamports: u64,
    blockhash: &[u8; 32],
) -> Result<Vec<u8>> {
    let key = signing_key(seed)?;
    let from = key.verifying_key().to_bytes();
    let to = parse_address(to)?;
    if from == to {
        return Err(bad("recipient", "sending to your own address"));
    }

    let mut dests = vec![(to, lamports)];
    if fee_lamports > 0 {
        let fee_to = parse_address(fee_to)?;

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

fn on_curve(bytes: &[u8; 32]) -> bool {
    CompressedEdwardsY(*bytes).decompress().is_some()
}

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

fn find_program_address(seeds: &[&[u8]], program: &[u8; 32]) -> [u8; 32] {
    for bump in (0u8..=255).rev() {
        let tail = [bump];
        let mut all: Vec<&[u8]> = seeds.to_vec();
        all.push(&tail);
        if let Some(addr) = create_program_address(&all, program) {
            return addr;
        }
    }

    [0u8; 32]
}

pub fn associated_token_account(owner: &[u8; 32], mint: &[u8; 32]) -> [u8; 32] {
    let token = parse_address(TOKEN_PROGRAM).expect("token program id is valid");
    let ata = parse_address(ATA_PROGRAM).expect("ata program id is valid");
    find_program_address(&[owner, &token, mint], &ata)
}

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

    let accounts: [[u8; 32]; 8] = [
        *owner,
        source_ata,
        dest_ata,
        *recipient,
        *mint,
        SYSTEM_PROGRAM,
        token,
        ata_prog,
    ];

    let mut msg = Vec::new();

    msg.push(1);
    msg.push(0);
    msg.push(5);

    push_compact_u16(&mut msg, accounts.len() as u16);
    for a in &accounts {
        msg.extend_from_slice(a);
    }

    msg.extend_from_slice(blockhash);

    push_compact_u16(&mut msg, 2);

    msg.push(7);
    push_compact_u16(&mut msg, 6);
    for i in [0u8, 2, 3, 4, 5, 6] {
        msg.push(i);
    }
    push_compact_u16(&mut msg, 1);
    msg.push(CREATE_ATA_IDEMPOTENT);

    msg.push(6);
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

pub fn signed_spl_transfer(
    seed: &[u8],
    to: &str,
    mint: &str,
    amount: u64,
    decimals: u8,
    blockhash: &[u8; 32],
) -> Result<Vec<u8>> {
    let key = signing_key(seed)?;
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
        let own = bs58::encode(signing_key(&seed).unwrap().verifying_key().to_bytes()).into_string();
        assert!(signed_transfer(&seed, &own, 1, &[7u8; 32]).is_err());
    }

    #[test]
    fn a_two_recipient_message_has_four_accounts_and_two_instructions() {
        let from = [1u8; 32];
        let to = [2u8; 32];
        let fee = [3u8; 32];
        let blockhash = [4u8; 32];
        let msg = build_message_multi(&from, &[(to, 900), (fee, 100)], &blockhash);

        assert_eq!(&msg[0..3], &[1, 0, 1]);
        assert_eq!(msg[3], 4);
        assert_eq!(&msg[4..36], &from);
        assert_eq!(&msg[36..68], &to);
        assert_eq!(&msg[68..100], &fee);
        assert_eq!(&msg[100..132], &SYSTEM_PROGRAM);

        assert_eq!(&msg[132..164], &blockhash);
        assert_eq!(msg[164], 2);
    }

    #[test]
    fn one_recipient_still_matches_the_original_layout() {

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

        assert_eq!(msg.len(), 150);
        assert_eq!(&msg[0..3], &[1, 0, 1]);
        assert_eq!(msg[3], 3, "three accounts");
        assert_eq!(&msg[4..36], &from);
        assert_eq!(&msg[36..68], &to);
        assert_eq!(&msg[68..100], &SYSTEM_PROGRAM);
        assert_eq!(&msg[100..132], &blockhash);

        let data = &msg[msg.len() - 12..];
        assert_eq!(&data[0..4], &TRANSFER_INSTRUCTION.to_le_bytes());
        assert_eq!(&data[4..12], &1_000_000u64.to_le_bytes());
    }

    #[test]
    fn real_keys_are_on_curve_and_atas_are_not() {

        let owner = signing_key(&test_seed()).unwrap().verifying_key().to_bytes();
        assert!(on_curve(&owner), "a real public key is on the curve");
        let mint = parse_address("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
        assert!(on_curve(&mint), "a mint address is a real key, on the curve");
        let ata = associated_token_account(&owner, &mint);
        assert!(!on_curve(&ata), "a derived token account is off the curve");
    }

    #[test]
    fn ata_is_deterministic_and_mint_specific() {
        let owner = signing_key(&test_seed()).unwrap().verifying_key().to_bytes();
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

        assert_eq!(&msg[0..3], &[1, 0, 5]);
        assert_eq!(msg[3], 8, "eight accounts");
        assert_eq!(&msg[4..36], &owner, "the signer comes first");

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

        let key = signing_key(&seed).unwrap().verifying_key();
        assert!(key.verify(message, &signature).is_ok());
    }
}
