//! The distributor's ChangeNOW credentials, compiled into the program.
//!
//! ChangeNOW requires an API key on every swap call, and the partner
//! commission that earns money is attributed to that key's account (the rate
//! is set once in the ChangeNOW partner dashboard, not per request). In a
//! wallet each person runs their own copy, so a key typed by the end user
//! would earn nothing for anyone. The key therefore belongs to whoever builds
//! and hands out the program, exactly as the donation addresses do, and is set
//! here, once, before building.
//!
//! # Obfuscation, and its honest limit
//!
//! The key below is stored transformed rather than in the clear, so it does not
//! fall out of a `strings` scan of the binary. That is a speed bump against
//! casual extraction and nothing more: a compiled-in secret can always be
//! recovered by someone determined, who need only watch the outgoing request
//! header or reverse the transform. The only real hiding place would be a relay
//! server, which this wallet does not have. For a referral key, whose worst
//! case is someone riding the referral or burning the rate limit, that trade is
//! fine. Do not put anything that must actually stay secret here.
//!
//! # Setting your key
//!
//! Get a free key (no KYC) by signing up at <https://changenow.io/affiliate>;
//! it appears under Profile details. Then run, with your key in the
//! environment:
//!
//! ```text
//! SF_CHANGENOW_KEY=your-key-here cargo test -p shinyflakes obfuscate_helper -- --ignored --nocapture
//! ```
//!
//! Paste the printed array into `KEY_OBF`. The commission percentage is set in
//! the ChangeNOW dashboard, so there is nothing else to configure here.

/// The obfuscated API key. Empty by default: swaps then reach ChangeNOW with no
/// key and are refused, until you set this. See the module docs.
const KEY_OBF: &[u8] = &[];

/// Reverses the transform in `obfuscate`: undo the reversal, then XOR and add
/// the offset back.
pub fn api_key() -> String {
    let plain: Vec<u8> = KEY_OBF
        .iter()
        .rev()
        .map(|b| (b ^ 42).wrapping_add(5))
        .collect();
    String::from_utf8(plain).unwrap_or_default()
}

/// The transform a key is stored under: offset down by five, XOR with 42, then
/// reverse the order. Kept next to `api_key` so the two never drift apart.
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
            // Temporarily stand in for KEY_OBF to prove decode inverts encode.
            let obf = obfuscate(sample);
            let plain: String = String::from_utf8(
                obf.iter().rev().map(|b| (b ^ 42).wrapping_add(5)).collect(),
            )
            .unwrap();
            assert_eq!(plain, sample);
        }
    }

    #[test]
    fn the_default_key_is_empty() {
        assert!(api_key().is_empty());
    }

    /// Not a test: a helper the distributor runs to obfuscate their own key.
    /// `SF_CHANGENOW_KEY=... cargo test obfuscate_helper -- --ignored --nocapture`
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
