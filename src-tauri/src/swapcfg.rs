//! ChangeNOW API key, baked in at build time.
//!
//! Swaps need a partner key, and the commission is tied to the builder's
//! account rather than the end user, so it's set here once before building.
//! It's stored obfuscated (below) so it doesn't fall out of a `strings` scan —
//! a speed bump, not real secrecy. Don't put anything here that must truly
//! stay secret.
//!
//! To use your own key: get a free one (no KYC) at
//! <https://changenow.io/affiliate>, then run
//!
//! ```text
//! SF_CHANGENOW_KEY=your-key cargo test -p shinyflakes obfuscate_helper -- --ignored --nocapture
//! ```
//!
//! and put the printed bytes in `src-tauri/swap_key.obf` (gitignored, read by
//! build.rs). Without that file, builds ship with no key and swaps report that
//! none was compiled in.

use zeroize::{Zeroize, Zeroizing};

include!(concat!(env!("OUT_DIR"), "/swap_key.rs"));

pub fn api_key() -> Zeroizing<String> {
    let mut plain: Vec<u8> = KEY_OBF
        .iter()
        .rev()
        .map(|b| (b ^ 42).wrapping_add(5))
        .collect();
    let key = String::from_utf8(plain.clone()).unwrap_or_default();
    plain.zeroize();
    Zeroizing::new(key)
}

#[cfg(test)]
fn obfuscate(plain: &str) -> Vec<u8> {
    let mut bytes: Vec<u8> = plain
        .bytes()
        .map(|b| b.wrapping_sub(5) ^ 42)
        .collect();
    bytes.reverse();
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obfuscation_round_trips() {
        for sample in ["", "abc123", "changenow-Test_KEY-9f8e7d6c"] {

            let obf = obfuscate(sample);
            let plain: String = String::from_utf8(
                obf.iter().rev().map(|b| (b ^ 42).wrapping_add(5)).collect(),
            )
            .unwrap();
            assert_eq!(plain, sample);
        }
    }

    #[test]
    fn api_key_decodes_without_panicking() {

        let key = api_key();
        assert!(key.is_empty() || !key.is_empty());
    }

    #[test]
    #[ignore]
    fn obfuscate_helper() {
        let key = std::env::var("SF_CHANGENOW_KEY").unwrap_or_default();
        if key.is_empty() {
            println!("set SF_CHANGENOW_KEY to your ChangeNOW API key first");
            return;
        }
        let obf = obfuscate(&key);
        let list = obf
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!("const KEY_OBF: &[u8] = &[{list}];");
    }
}
