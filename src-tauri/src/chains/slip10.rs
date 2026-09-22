//! SLIP-0010 ed25519 derivation.

use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::{Zeroize, ZeroizeOnDrop};

type HmacSha512 = Hmac<Sha512>;

const HARDENED: u32 = 0x8000_0000;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Node {
    pub key: [u8; 32],
    pub chain_code: [u8; 32],
}

fn split(bytes: &[u8]) -> Node {
    let mut key = [0u8; 32];
    let mut chain_code = [0u8; 32];
    key.copy_from_slice(&bytes[..32]);
    chain_code.copy_from_slice(&bytes[32..64]);
    Node { key, chain_code }
}

pub fn master(seed: &[u8]) -> Node {
    let mut mac =
        HmacSha512::new_from_slice(b"ed25519 seed").expect("hmac takes a key of any length");
    mac.update(seed);
    split(&mac.finalize().into_bytes())
}

pub fn child(parent: &Node, index: u32) -> Node {
    let mut mac =
        HmacSha512::new_from_slice(&parent.chain_code).expect("hmac takes a key of any length");
    mac.update(&[0u8]);
    mac.update(&parent.key);
    mac.update(&(index | HARDENED).to_be_bytes());
    split(&mac.finalize().into_bytes())
}

pub fn derive(seed: &[u8], path: &[u32]) -> Node {
    let mut node = master(seed);
    for step in path {
        node = child(&node, *step);
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ];

    #[test]
    fn matches_slip10_master() {
        let node = master(&SEED);
        assert_eq!(
            hex(&node.chain_code),
            "90046a93de5380a72b5e45010748567d5ea02bbf6522f979e05c0d8d8ca9fffb"
        );
        assert_eq!(
            hex(&node.key),
            "2b4be7f19ee27bbf30c667b642d5f4aa69fd169872f8fc3059c08ebae2eb19e7"
        );
    }

    #[test]
    fn matches_slip10_first_child() {
        let node = derive(&SEED, &[0]);
        assert_eq!(
            hex(&node.chain_code),
            "8b59aa11380b624e81507a27fedda59fea6d0b779a778918a2fd3590e16e9c69"
        );
        assert_eq!(
            hex(&node.key),
            "68e0fe46dfb67e368c75379acec591dad19df3cde26e63b93a8e704f1dade7a3"
        );
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
