//! Bitcoin transaction building.

use bip32::{DerivationPath, XPrv};
use k256::ecdsa::signature::hazmat::PrehashSigner;
use k256::ecdsa::{Signature, SigningKey};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

use crate::error::{Result, WalletError};

const SIGHASH_ALL: u32 = 1;

const SEQUENCE: u32 = 0xffff_fffd;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chain {
    Bitcoin,
    Litecoin,
}

impl Chain {
    pub fn hrp(self) -> &'static str {
        match self {
            Chain::Bitcoin => "bc",
            Chain::Litecoin => "ltc",
        }
    }

    fn legacy_versions(self) -> (u8, u8) {
        match self {
            Chain::Bitcoin => (0x00, 0x05),

            Chain::Litecoin => (0x30, 0x32),
        }
    }

    pub fn path(self) -> &'static str {
        match self {
            Chain::Bitcoin => super::BTC_PATH,
            Chain::Litecoin => super::LTC_PATH,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Utxo {

    pub txid: [u8; 32],
    pub vout: u32,
    pub value: u64,

    pub key_index: u32,
}

fn bad(what: &str, e: impl std::fmt::Display) -> WalletError {
    WalletError::Derivation(format!("bitcoin {what}: {e}"))
}

fn sha256d(data: &[u8]) -> [u8; 32] {
    let once = Sha256::digest(data);
    let twice = Sha256::digest(once);
    let mut out = [0u8; 32];
    out.copy_from_slice(&twice);
    out
}

fn hash160(data: &[u8]) -> [u8; 20] {
    let out = Ripemd160::digest(Sha256::digest(data));
    let mut result = [0u8; 20];
    result.copy_from_slice(&out);
    result
}

fn varint(value: u64, out: &mut Vec<u8>) {
    match value {
        0..=0xfc => out.push(value as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(value as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(value as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

fn push_bytes(data: &[u8], out: &mut Vec<u8>) {
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

pub fn script_pubkey_for(address: &str, chain: Chain) -> Result<Vec<u8>> {
    let trimmed = address.trim();

    if let Ok((hrp, _version, program)) = bech32::segwit::decode(trimmed) {
        if hrp.as_str() != chain.hrp() {
            return Err(bad(
                "address",
                format!("that address is not on this chain (prefix {})", hrp.as_str()),
            ));
        }

        let mut script = Vec::with_capacity(2 + program.len());
        script.push(0x00);
        push_bytes(&program, &mut script);
        return Ok(script);
    }

    let raw: Vec<u8> = bs58::decode(trimmed)
        .with_check(None)
        .into_vec()
        .map_err(|e| bad("address", e))?;
    if raw.len() != 21 {
        return Err(bad("address", "unexpected length"));
    }

    let (p2pkh, p2sh) = chain.legacy_versions();
    let hash = &raw[1..];

    if raw[0] == p2pkh {

        let mut script = vec![0x76, 0xa9];
        push_bytes(hash, &mut script);
        script.extend_from_slice(&[0x88, 0xac]);
        Ok(script)
    } else if raw[0] == p2sh || (chain == Chain::Litecoin && raw[0] == 0x05) {

        let mut script = vec![0xa9];
        push_bytes(hash, &mut script);
        script.push(0x87);
        Ok(script)
    } else {
        Err(bad(
            "address",
            format!("unrecognised address version {}", raw[0]),
        ))
    }
}

pub struct Keys {
    pub signing: SigningKey,
    pub pubkey: Vec<u8>,
    pub pubkey_hash: [u8; 20],
}

pub fn keys_at(seed: &[u8], chain: Chain, index: u32) -> Result<Keys> {

    let base = chain.path().rsplit_once('/').map(|(head, _)| head).unwrap_or(chain.path());
    let full = format!("{base}/{index}");

    let path: DerivationPath = full.parse().map_err(|e| bad("path", e))?;
    let xprv = XPrv::derive_from_path(seed, &path).map_err(|e| bad("derive", e))?;

    let signing = xprv.private_key().clone();
    let point = signing.verifying_key().to_encoded_point(true);
    let pubkey = point.as_bytes().to_vec();
    let pubkey_hash = hash160(&pubkey);

    Ok(Keys {
        signing,
        pubkey,
        pubkey_hash,
    })
}

pub fn own_address(pubkey_hash: &[u8; 20], chain: Chain) -> Result<String> {
    let hrp = bech32::Hrp::parse(chain.hrp()).map_err(|e| bad("hrp", e))?;
    bech32::segwit::encode_v0(hrp, pubkey_hash).map_err(|e| bad("bech32", e))
}

pub fn own_script(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut script = vec![0x00];
    push_bytes(pubkey_hash, &mut script);
    script
}

fn script_code(pubkey_hash: &[u8; 20]) -> Vec<u8> {
    let mut script = vec![0x76, 0xa9];
    push_bytes(pubkey_hash, &mut script);
    script.extend_from_slice(&[0x88, 0xac]);
    script
}

#[derive(Debug)]
pub struct Output {
    pub script: Vec<u8>,
    pub value: u64,
}

#[allow(clippy::too_many_arguments)]
pub fn sighash(
    version: u32,
    inputs: &[Utxo],
    outputs: &[Output],
    index: usize,
    pubkey_hash: &[u8; 20],
    sequence: u32,
    locktime: u32,
) -> [u8; 32] {
    let mut prevouts = Vec::new();
    let mut sequences = Vec::new();
    for input in inputs {
        prevouts.extend_from_slice(&input.txid);
        prevouts.extend_from_slice(&input.vout.to_le_bytes());
        sequences.extend_from_slice(&sequence.to_le_bytes());
    }

    let mut outs = Vec::new();
    for output in outputs {
        outs.extend_from_slice(&output.value.to_le_bytes());
        push_bytes(&output.script, &mut outs);
    }

    let hash_prevouts = sha256d(&prevouts);
    let hash_sequence = sha256d(&sequences);
    let hash_outputs = sha256d(&outs);

    let spending = &inputs[index];
    let code = script_code(pubkey_hash);

    let mut pre = Vec::new();
    pre.extend_from_slice(&version.to_le_bytes());
    pre.extend_from_slice(&hash_prevouts);
    pre.extend_from_slice(&hash_sequence);
    pre.extend_from_slice(&spending.txid);
    pre.extend_from_slice(&spending.vout.to_le_bytes());
    push_bytes(&code, &mut pre);
    pre.extend_from_slice(&spending.value.to_le_bytes());
    pre.extend_from_slice(&sequence.to_le_bytes());
    pre.extend_from_slice(&hash_outputs);
    pre.extend_from_slice(&locktime.to_le_bytes());
    pre.extend_from_slice(&SIGHASH_ALL.to_le_bytes());

    sha256d(&pre)
}

pub fn estimated_vsize(inputs: usize, outputs: usize) -> u64 {
    let base = 10 + 1 + 1;
    (base + inputs * 68 + outputs * 43) as u64
}

pub fn build_signed(
    keyring: &[Keys],
    inputs: &[Utxo],
    outputs: &[Output],
    locktime: u32,
) -> Result<Vec<u8>> {
    if inputs.is_empty() {
        return Err(bad("inputs", "nothing to spend"));
    }
    if outputs.is_empty() {
        return Err(bad("outputs", "nothing to pay"));
    }

    let version: u32 = 2;
    let mut witnesses: Vec<Vec<Vec<u8>>> = Vec::with_capacity(inputs.len());

    for index in 0..inputs.len() {

        let owner = keyring
            .get(inputs[index].key_index as usize)
            .ok_or_else(|| bad("signing", "no key for one of the outputs being spent"))?;

        let digest = sighash(
            version,
            inputs,
            outputs,
            index,
            &owner.pubkey_hash,
            SEQUENCE,
            locktime,
        );

        let signature: Signature = owner
            .signing
            .sign_prehash(&digest)
            .map_err(|e| bad("signing", e))?;

        let normalised = signature.normalize_s().unwrap_or(signature);

        let mut der = normalised.to_der().as_bytes().to_vec();
        der.push(SIGHASH_ALL as u8);

        witnesses.push(vec![der, owner.pubkey.clone()]);
    }

    let mut tx = Vec::new();
    tx.extend_from_slice(&version.to_le_bytes());

    tx.push(0x00);
    tx.push(0x01);

    varint(inputs.len() as u64, &mut tx);
    for input in inputs {
        tx.extend_from_slice(&input.txid);
        tx.extend_from_slice(&input.vout.to_le_bytes());

        tx.push(0x00);
        tx.extend_from_slice(&SEQUENCE.to_le_bytes());
    }

    varint(outputs.len() as u64, &mut tx);
    for output in outputs {
        tx.extend_from_slice(&output.value.to_le_bytes());
        push_bytes(&output.script, &mut tx);
    }

    for witness in &witnesses {
        varint(witness.len() as u64, &mut tx);
        for item in witness {
            push_bytes(item, &mut tx);
        }
    }

    tx.extend_from_slice(&locktime.to_le_bytes());
    Ok(tx)
}

pub fn txid(signed: &[u8]) -> String {

    let mut stripped = Vec::with_capacity(signed.len());
    stripped.extend_from_slice(&signed[0..4]);

    let mut cursor = 6;
    let (count, used) = read_varint(&signed[cursor..]);
    varint(count, &mut stripped);
    cursor += used;

    for _ in 0..count {
        stripped.extend_from_slice(&signed[cursor..cursor + 36]);
        cursor += 36;
        let (len, used) = read_varint(&signed[cursor..]);
        cursor += used;
        varint(len, &mut stripped);
        stripped.extend_from_slice(&signed[cursor..cursor + len as usize]);
        cursor += len as usize;
        stripped.extend_from_slice(&signed[cursor..cursor + 4]);
        cursor += 4;
    }

    let (out_count, used) = read_varint(&signed[cursor..]);
    varint(out_count, &mut stripped);
    cursor += used;
    for _ in 0..out_count {
        stripped.extend_from_slice(&signed[cursor..cursor + 8]);
        cursor += 8;
        let (len, used) = read_varint(&signed[cursor..]);
        cursor += used;
        varint(len, &mut stripped);
        stripped.extend_from_slice(&signed[cursor..cursor + len as usize]);
        cursor += len as usize;
    }

    stripped.extend_from_slice(&signed[signed.len() - 4..]);

    let hash = sha256d(&stripped);
    hash.iter().rev().map(|b| format!("{b:02x}")).collect()
}

fn read_varint(data: &[u8]) -> (u64, usize) {
    match data[0] {
        0xfd => (u16::from_le_bytes([data[1], data[2]]) as u64, 3),
        0xfe => (
            u32::from_le_bytes([data[1], data[2], data[3], data[4]]) as u64,
            5,
        ),
        0xff => (
            u64::from_le_bytes([
                data[1], data[2], data[3], data[4], data[5], data[6], data[7], data[8],
            ]),
            9,
        ),
        n => (n as u64, 1),
    }
}

pub const DUST: u64 = 546;

pub struct Plan {
    pub inputs: Vec<Utxo>,
    pub outputs: Vec<Output>,
    pub fee: u64,

    #[allow(dead_code)]
    pub change: u64,
}

#[allow(dead_code)]
pub fn select(
    utxos: &[Utxo],
    dest_script: Vec<u8>,
    amount: u64,
    change_script: Vec<u8>,
    fee_rate: f64,
) -> Result<Plan> {
    if amount < DUST {
        return Err(WalletError::Funds(format!(
            "{amount} is below the dust limit of {DUST}, so the network would reject it."
        )));
    }
    select_outputs(
        utxos,
        vec![Output { script: dest_script, value: amount }],
        change_script,
        fee_rate,
    )
}

pub fn select_outputs(
    utxos: &[Utxo],
    fixed: Vec<Output>,
    change_script: Vec<u8>,
    fee_rate: f64,
) -> Result<Plan> {
    let out_value: u64 = fixed.iter().map(|o| o.value).sum();
    let n = fixed.len();

    let fee_for = |inputs: usize, outputs: usize| {
        (estimated_vsize(inputs, outputs) as f64 * fee_rate).ceil() as u64
    };

    let mut chosen: Vec<Utxo> = Vec::new();
    let mut total: u64 = 0;

    for utxo in utxos {
        chosen.push(utxo.clone());
        total += utxo.value;

        let fee = fee_for(chosen.len(), n + 1);
        if total >= out_value + fee {
            let change = total - out_value - fee;
            if change >= DUST {
                let mut outputs = fixed;
                outputs.push(Output { script: change_script, value: change });
                return Ok(Plan { inputs: chosen, outputs, fee, change });
            }
        }

        let lean = fee_for(chosen.len(), n);
        if total >= out_value + lean {
            return Ok(Plan {
                inputs: chosen,
                outputs: fixed,
                fee: total - out_value,
                change: 0,
            });
        }
    }

    let held: u64 = utxos.iter().map(|u| u.value).sum();
    let needed = out_value + fee_for(utxos.len().max(1), n + 1);
    Err(WalletError::Funds(format!(
        "This needs about {needed} including fees, but only {held} is confirmed and spendable."
    )))
}

pub fn max_sendable(utxos: &[Utxo], fee_rate: f64) -> u64 {
    if utxos.is_empty() {
        return 0;
    }
    let total: u64 = utxos.iter().map(|u| u.value).sum();
    let fee = (estimated_vsize(utxos.len(), 1) as f64 * fee_rate).ceil() as u64;
    total.saturating_sub(fee)
}

#[derive(Debug)]
pub struct FragmentPlan {
    pub inputs: Vec<Utxo>,
    pub outputs: Vec<Output>,
    pub fee: u64,
    pub per_piece: u64,
    pub pieces: u32,

    pub change: u64,

    pub sources: usize,
}

pub fn fragment(
    utxos: &[Utxo],
    amount: u64,
    pieces: u32,
    piece_scripts: &[Vec<u8>],
    change_script: Vec<u8>,
    fee_rate: f64,
) -> Result<FragmentPlan> {
    if pieces < 2 {
        return Err(WalletError::Funds(
            "Splitting into fewer than two pieces would not be a split.".into(),
        ));
    }
    if piece_scripts.len() < pieces as usize {
        return Err(bad("fragment", "not enough addresses for the requested pieces"));
    }

    let per_piece = amount / pieces as u64;
    if per_piece < DUST {
        return Err(WalletError::Funds(format!(
            "Each of the {pieces} pieces would hold {per_piece}, below the dust limit of {DUST}.              Split into fewer pieces or use a larger amount."
        )));
    }

    let remainder = amount - per_piece * pieces as u64;

    let fee_for = |inputs: usize, outputs: usize| {
        (estimated_vsize(inputs, outputs) as f64 * fee_rate).ceil() as u64
    };

    let build_outputs = |include_change: bool, change: u64| {
        let mut outputs: Vec<Output> = (0..pieces as usize)
            .map(|i| Output {
                script: piece_scripts[i].clone(),
                value: if i + 1 == pieces as usize {
                    per_piece + remainder
                } else {
                    per_piece
                },
            })
            .collect();
        if include_change {
            outputs.push(Output {
                script: change_script.clone(),
                value: change,
            });
        }
        outputs
    };

    let mut chosen: Vec<Utxo> = Vec::new();
    let mut total: u64 = 0;

    for utxo in utxos {
        chosen.push(utxo.clone());
        total += utxo.value;

        let with_change = fee_for(chosen.len(), pieces as usize + 1);
        if total >= amount + with_change {
            let change = total - amount - with_change;
            if change >= DUST {
                let sources = distinct_sources(&chosen);
                return Ok(FragmentPlan {
                    inputs: chosen,
                    outputs: build_outputs(true, change),
                    fee: with_change,
                    per_piece,
                    pieces,
                    change,
                    sources,
                });
            }
        }

        let lean = fee_for(chosen.len(), pieces as usize);
        if total >= amount + lean {
            let sources = distinct_sources(&chosen);
            return Ok(FragmentPlan {
                inputs: chosen,
                outputs: build_outputs(false, 0),
                fee: total - amount,
                per_piece,
                pieces,
                change: 0,
                sources,
            });
        }
    }

    let held: u64 = utxos.iter().map(|u| u.value).sum();
    Err(WalletError::Funds(format!(
        "Not enough balance. Splitting {amount} into {pieces} pieces needs about {}          including fees, and only {held} is confirmed and spendable.",
        amount + fee_for(utxos.len().max(1), pieces as usize + 1)
    )))
}

#[allow(dead_code)]
pub fn plan_with(
    inputs: &[Utxo],
    dest_script: Vec<u8>,
    amount: u64,
    change_script: Vec<u8>,
    fee_rate: f64,
) -> Result<Plan> {
    if amount < DUST {
        return Err(WalletError::Funds(format!(
            "{amount} is below the dust limit of {DUST}, so the network would reject it."
        )));
    }
    plan_with_outputs(
        inputs,
        vec![Output { script: dest_script, value: amount }],
        change_script,
        fee_rate,
    )
}

pub fn plan_with_outputs(
    inputs: &[Utxo],
    fixed: Vec<Output>,
    change_script: Vec<u8>,
    fee_rate: f64,
) -> Result<Plan> {
    if inputs.is_empty() {
        return Err(WalletError::Funds("No coins were chosen to spend.".into()));
    }

    let out_value: u64 = fixed.iter().map(|o| o.value).sum();
    let n = fixed.len();
    let total: u64 = inputs.iter().map(|u| u.value).sum();
    let fee_for = |outputs: usize| {
        (estimated_vsize(inputs.len(), outputs) as f64 * fee_rate).ceil() as u64
    };

    let with_change = fee_for(n + 1);
    if total >= out_value + with_change {
        let change = total - out_value - with_change;
        if change >= DUST {
            let mut outputs = fixed;
            outputs.push(Output { script: change_script, value: change });
            return Ok(Plan { inputs: inputs.to_vec(), outputs, fee: with_change, change });
        }
    }

    let lean = fee_for(n);
    if total >= out_value + lean {
        return Ok(Plan {
            inputs: inputs.to_vec(),
            outputs: fixed,
            fee: total - out_value,
            change: 0,
        });
    }

    Err(WalletError::Funds(format!(
        "The chosen coins hold {total}, which does not cover {out_value} plus a fee of about {lean}."
    )))
}

pub fn consolidate(utxos: &[Utxo], dest_script: Vec<u8>, fee_rate: f64) -> Result<Plan> {
    if utxos.is_empty() {
        return Err(WalletError::Funds("There is nothing to combine.".into()));
    }

    let total: u64 = utxos.iter().map(|u| u.value).sum();
    let fee = (estimated_vsize(utxos.len(), 1) as f64 * fee_rate).ceil() as u64;

    if total <= fee + DUST {
        return Err(WalletError::Funds(format!(
            "Combining these {} outputs would cost {fee} in fees, which is more than the              {total} they hold.",
            utxos.len()
        )));
    }

    Ok(Plan {
        inputs: utxos.to_vec(),
        outputs: vec![Output {
            script: dest_script,
            value: total - fee,
        }],
        fee,
        change: 0,
    })
}

pub const MAX_STANDARD_TX_VSIZE: u64 = 100_000;

pub fn max_pieces(total: u64, input_count: usize) -> u32 {
    let by_dust = total / DUST;

    let overhead = estimated_vsize(input_count.max(1), 1);
    let room = MAX_STANDARD_TX_VSIZE.saturating_sub(overhead);
    let by_size = room / 43;

    by_dust.min(by_size).min(u32::MAX as u64) as u32
}

fn distinct_sources(inputs: &[Utxo]) -> usize {
    let mut seen: Vec<u32> = inputs.iter().map(|u| u.key_index).collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn matches_the_bip143_p2wpkh_vector() {

        let txid0 = unhex("fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f");
        let txid1 = unhex("ef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a");

        let inputs = vec![
            Utxo {
                txid: txid0.try_into().unwrap(),
                vout: 0,
                value: 625_000_000,
                key_index: 0,
            },
            Utxo {
                txid: txid1.try_into().unwrap(),
                vout: 1,
                value: 600_000_000,
                key_index: 0,
            },
        ];

        let outputs = vec![
            Output {
                script: unhex("76a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac"),
                value: 112_340_000,
            },
            Output {
                script: unhex("76a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac"),
                value: 223_450_000,
            },
        ];

        let pubkey = unhex("025476c2e83188368da1ff3e292e7acafcdb3566bb0ad253f62fc70f07aeee6357");
        let pubkey_hash = hash160(&pubkey);
        assert_eq!(hex(&pubkey_hash), "1d0f172a0ecb48aee1be1f2687d2963ae33f71a1");

        let mut prevouts = Vec::new();
        for input in &inputs {
            prevouts.extend_from_slice(&input.txid);
            prevouts.extend_from_slice(&input.vout.to_le_bytes());
        }
        assert_eq!(
            hex(&sha256d(&prevouts)),
            "96b827c8483d4e9b96712b6713a7b68d6e8003a781feba36c31143470b4efd37"
        );

        let mut outs = Vec::new();
        for output in &outputs {
            outs.extend_from_slice(&output.value.to_le_bytes());
            push_bytes(&output.script, &mut outs);
        }
        assert_eq!(
            hex(&sha256d(&outs)),
            "863ef3e1a92afbfdb97f31ad0fc7683ee943e9abcf2501590ff8f6551f47e5e5"
        );

        let mut sequences = Vec::new();
        sequences.extend_from_slice(&0xffff_ffeeu32.to_le_bytes());
        sequences.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        assert_eq!(
            hex(&sha256d(&sequences)),
            "52b0a642eea2fb7ae638c36f6252b6750293dbe574a806984b8e4d8548339a3b"
        );

        let mut pre = Vec::new();
        pre.extend_from_slice(&1u32.to_le_bytes());
        pre.extend_from_slice(&sha256d(&prevouts));
        pre.extend_from_slice(&sha256d(&sequences));
        pre.extend_from_slice(&inputs[1].txid);
        pre.extend_from_slice(&inputs[1].vout.to_le_bytes());
        push_bytes(&script_code(&pubkey_hash), &mut pre);
        pre.extend_from_slice(&inputs[1].value.to_le_bytes());
        pre.extend_from_slice(&0xffff_ffffu32.to_le_bytes());
        pre.extend_from_slice(&sha256d(&outs));
        pre.extend_from_slice(&0x0000_0011u32.to_le_bytes());
        pre.extend_from_slice(&SIGHASH_ALL.to_le_bytes());

        assert_eq!(
            hex(&sha256d(&pre)),
            "c37af31116d1b27caf68aae9e3ac82f1477929014d5b917657d0eb49478cb670"
        );
    }

    #[test]
    fn varints_match_the_wire_format() {
        let cases: [(u64, &str); 5] = [
            (0, "00"),
            (0xfc, "fc"),
            (0xfd, "fdfd00"),
            (0x1_0000, "fe00000100"),
            (0x1_0000_0000, "ff0000000001000000"),
        ];
        for (value, expected) in cases {
            let mut out = Vec::new();
            varint(value, &mut out);
            assert_eq!(hex(&out), expected, "varint {value}");
        }
    }

    #[test]
    fn builds_the_right_script_for_each_address_kind() {

        let btc = script_pubkey_for("bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", Chain::Bitcoin)
            .unwrap();
        assert_eq!(btc[0], 0x00);
        assert_eq!(btc[1], 20);
        assert_eq!(btc.len(), 22);

        let ltc = script_pubkey_for(
            "ltc1qjmxnz78nmc8nq77wuxh25n2es7rzm5c2rkk4wh",
            Chain::Litecoin,
        )
        .unwrap();
        assert_eq!(ltc.len(), 22);

        let legacy =
            script_pubkey_for("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2", Chain::Bitcoin).unwrap();
        assert_eq!(legacy[0], 0x76);
        assert_eq!(legacy[1], 0xa9);
        assert_eq!(legacy.len(), 25);
        assert_eq!(legacy[23], 0x88);
        assert_eq!(legacy[24], 0xac);
    }

    #[test]
    fn refuses_an_address_from_the_wrong_chain() {

        let wrong = script_pubkey_for(
            "ltc1qjmxnz78nmc8nq77wuxh25n2es7rzm5c2rkk4wh",
            Chain::Bitcoin,
        );
        assert!(wrong.is_err());
    }

    #[test]
    fn refuses_rubbish() {
        assert!(script_pubkey_for("not an address", Chain::Bitcoin).is_err());
        assert!(script_pubkey_for("", Chain::Bitcoin).is_err());
    }

    fn utxo(value: u64, vout: u32) -> Utxo {
        Utxo {
            txid: [vout as u8; 32],
            vout,
            value,
            key_index: 0,
        }
    }

    fn dest() -> Vec<u8> {
        script_pubkey_for("bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu", Chain::Bitcoin).unwrap()
    }

    #[test]
    fn a_fee_output_is_carried_alongside_the_payment() {
        let utxos = vec![utxo(1_000_000, 0)];
        let fixed = vec![
            Output { script: dest(), value: 400_000 },
            Output { script: dest(), value: 4_000 },
        ];
        let plan = select_outputs(&utxos, fixed, dest(), 5.0).unwrap();

        assert_eq!(plan.outputs.len(), 3);
        assert_eq!(plan.outputs[0].value, 400_000);
        assert_eq!(plan.outputs[1].value, 4_000);
        let spent: u64 = plan.inputs.iter().map(|i| i.value).sum();
        let paid: u64 = plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(spent, paid + plan.fee);
    }

    #[test]
    fn selection_covers_the_amount_and_the_fee() {
        let utxos = vec![utxo(100_000, 0), utxo(50_000, 1)];
        let plan = select(&utxos, dest(), 60_000, dest(), 5.0).unwrap();

        let spent: u64 = plan.inputs.iter().map(|i| i.value).sum();
        let paid: u64 = plan.outputs.iter().map(|o| o.value).sum();

        assert_eq!(spent, paid + plan.fee);
        assert_eq!(plan.outputs[0].value, 60_000);
    }

    #[test]
    fn selection_prefers_fewer_inputs() {

        let utxos = vec![utxo(100_000, 0), utxo(50_000, 1)];
        let plan = select(&utxos, dest(), 60_000, dest(), 5.0).unwrap();
        assert_eq!(plan.inputs.len(), 1);
    }

    #[test]
    fn dust_change_goes_to_the_fee_instead_of_an_output() {

        let utxos = vec![utxo(61_000, 0)];
        let plan = select(&utxos, dest(), 60_000, dest(), 1.0).unwrap();
        if plan.change == 0 {
            assert_eq!(plan.outputs.len(), 1);
            assert_eq!(plan.inputs[0].value, plan.outputs[0].value + plan.fee);
        } else {
            assert!(plan.change >= DUST);
        }
    }

    #[test]
    fn refuses_when_the_balance_cannot_cover_the_fee() {
        let utxos = vec![utxo(60_100, 0)];

        let result = select(&utxos, dest(), 60_000, dest(), 500.0);
        assert!(result.is_err());
    }

    #[test]
    fn refuses_dust_payments() {
        let utxos = vec![utxo(100_000, 0)];
        assert!(select(&utxos, dest(), 100, dest(), 5.0).is_err());
    }

    #[test]
    fn max_sendable_leaves_nothing_behind() {
        let utxos = vec![utxo(100_000, 0), utxo(50_000, 1)];
        let max = max_sendable(&utxos, 5.0);
        let plan = select(&utxos, dest(), max, dest(), 5.0).unwrap();
        assert_eq!(plan.change, 0, "sending the maximum should leave no change");
    }

    fn scripts(n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|_| dest()).collect()
    }

    #[test]
    fn fragmenting_splits_evenly_and_balances() {
        let utxos = vec![utxo(1_000_000, 0)];
        let plan = fragment(&utxos, 400_000, 4, &scripts(4), dest(), 5.0).unwrap();

        assert_eq!(plan.pieces, 4);
        assert_eq!(plan.per_piece, 100_000);

        let spent: u64 = plan.inputs.iter().map(|i| i.value).sum();
        let paid: u64 = plan.outputs.iter().map(|o| o.value).sum();

        assert_eq!(spent, paid + plan.fee);

        assert_eq!(plan.outputs.len(), 5);
        for output in plan.outputs.iter().take(4) {
            assert_eq!(output.value, 100_000);
        }
    }

    #[test]
    fn rounding_remainder_goes_to_the_last_piece() {
        let utxos = vec![utxo(1_000_000, 0)];

        let plan = fragment(&utxos, 100_001, 3, &scripts(3), dest(), 2.0).unwrap();

        let pieces: Vec<u64> = plan.outputs.iter().take(3).map(|o| o.value).collect();
        assert_eq!(pieces.iter().sum::<u64>(), 100_001, "no satoshi may be lost");
        assert_eq!(pieces[2], pieces[0] + 2);
    }

    #[test]
    fn refuses_pieces_that_would_be_dust() {
        let utxos = vec![utxo(1_000_000, 0)];

        assert!(fragment(&utxos, 1_000, 10, &scripts(10), dest(), 2.0).is_err());
    }

    #[test]
    fn refuses_a_single_piece() {
        let utxos = vec![utxo(1_000_000, 0)];
        assert!(fragment(&utxos, 100_000, 1, &scripts(1), dest(), 2.0).is_err());
    }

    #[test]
    fn reports_how_many_sub_wallets_were_combined() {

        let mut a = utxo(60_000, 0);
        a.key_index = 0;
        let mut b = utxo(60_000, 1);
        b.key_index = 3;

        let plan = fragment(&[a, b], 100_000, 2, &scripts(2), dest(), 2.0).unwrap();
        assert_eq!(plan.inputs.len(), 2);
        assert_eq!(plan.sources, 2, "both sub-wallets had to be drawn from");
    }

    #[test]
    fn says_when_there_is_simply_not_enough() {
        let utxos = vec![utxo(50_000, 0)];
        let error = fragment(&utxos, 1_000_000, 4, &scripts(4), dest(), 2.0).unwrap_err();
        assert!(format!("{error}").contains("Not enough balance"));
    }

    #[test]
    fn more_pieces_cost_more_in_fees() {
        let utxos = vec![utxo(10_000_000, 0)];
        let few = fragment(&utxos, 1_000_000, 2, &scripts(2), dest(), 10.0).unwrap();
        let many = fragment(&utxos, 1_000_000, 8, &scripts(8), dest(), 10.0).unwrap();
        assert!(many.fee > few.fee, "eight outputs should cost more than two");
    }

    #[test]
    fn plan_with_spends_exactly_what_was_chosen() {
        let picked = vec![utxo(60_000, 0), utxo(60_000, 1)];
        let plan = plan_with(&picked, dest(), 100_000, dest(), 4.0).unwrap();

        assert_eq!(plan.inputs.len(), 2, "both chosen coins are spent");
        let spent: u64 = plan.inputs.iter().map(|i| i.value).sum();
        let paid: u64 = plan.outputs.iter().map(|o| o.value).sum();
        assert_eq!(spent, paid + plan.fee);
    }

    #[test]
    fn plan_with_does_not_reach_for_unchosen_coins() {

        let picked = vec![utxo(1_000, 0)];
        assert!(plan_with(&picked, dest(), 100_000, dest(), 4.0).is_err());
    }

    #[test]
    fn plan_with_refuses_an_empty_choice() {
        assert!(plan_with(&[], dest(), 1_000, dest(), 4.0).is_err());
    }

    #[test]
    fn consolidating_sweeps_everything_into_one() {
        let utxos = vec![utxo(100_000, 0), utxo(50_000, 1), utxo(25_000, 2)];
        let plan = consolidate(&utxos, dest(), 5.0).unwrap();

        assert_eq!(plan.inputs.len(), 3, "every output is spent");
        assert_eq!(plan.outputs.len(), 1, "into a single one");

        let spent: u64 = plan.inputs.iter().map(|i| i.value).sum();
        assert_eq!(spent, plan.outputs[0].value + plan.fee);
    }

    #[test]
    fn refuses_to_combine_when_the_fee_would_eat_it() {
        let utxos = vec![utxo(600, 0)];
        assert!(consolidate(&utxos, dest(), 500.0).is_err());
    }

    #[test]
    fn refuses_to_combine_nothing() {
        assert!(consolidate(&[], dest(), 5.0).is_err());
    }

    #[test]
    fn piece_ceiling_respects_both_limits() {

        assert_eq!(max_pieces(10_000, 1) as u64, 10_000 / DUST);

        let big = max_pieces(100_000_000_000, 1);
        assert!(big > 1_500, "size limit allows plenty: {big}");
        assert!(big < 2_500, "but not unbounded: {big}");
    }

    #[test]
    fn fee_estimate_grows_with_the_transaction() {
        assert!(estimated_vsize(1, 2) > estimated_vsize(1, 1));
        assert!(estimated_vsize(2, 2) > estimated_vsize(1, 2));

        let small = estimated_vsize(1, 1);
        assert!((100..130).contains(&small), "unexpected estimate: {small}");
    }
}
