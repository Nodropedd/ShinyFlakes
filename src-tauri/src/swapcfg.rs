//! The distributor's Trocador credentials, compiled into the program.
//!
//! Trocador requires an API key (a keyless request is rejected with 401), and
//! the markup that earns commission is attributed to that key's account. In a
//! wallet each person runs their own copy, so a key or markup typed by the end
//! user would earn nothing for anyone. The key therefore belongs to whoever
//! builds and hands out the program, exactly as the donation addresses do, and
//! is set here, once, before building.
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
//! Run, with your key in the environment:
//!
//! ```text
//! SF_TROCADOR_KEY=your-key-here cargo test -p shinyflakes obfuscate_helper -- --ignored --nocapture
//! ```
//!
//! Paste the printed array into `KEY_OBF`, and set `MARKUP` to the percentage
//! you want added on top of every rate.

/// The obfuscated API key. Empty by default: swaps then reach Trocador with no
/// key and are refused, until you set this. See the module docs.
const KEY_OBF: &[u8] = &[];

/// The percentage added on top of the rate, paid to the key's account. Zero is
/// none. This is the distributor's, not the end user's.
pub const MARKUP: f64 = 0.0;

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

pub fn markup() -> f64 {
    MARKUP
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
        for sample in ["", "abc123", "trocador-Test_KEY-9f8e7d6c"] {
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
    /// `SF_TROCADOR_KEY=... cargo test obfuscate_helper -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn obfuscate_helper() {
        let key = std::env::var("SF_TROCADOR_KEY").unwrap_or_default();
        if key.is_empty() {
            println!("set SF_TROCADOR_KEY to your Trocador API key first");
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
