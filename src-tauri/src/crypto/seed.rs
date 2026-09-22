//! BIP-39 seed handling.

use bip39::{Language, Mnemonic};
use zeroize::Zeroizing;

use crate::error::{Result, WalletError};

pub const SEED_LEN: usize = 64;
pub const NEW_WALLET_WORDS: usize = 24;

pub fn generate() -> Result<Zeroizing<String>> {
    let mnemonic = Mnemonic::generate_in(Language::English, NEW_WALLET_WORDS)
        .map_err(|_| WalletError::InvalidMnemonic)?;
    Ok(Zeroizing::new(mnemonic.to_string()))
}

pub fn parse(phrase: &str) -> Result<Mnemonic> {
    let normalised = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
    Mnemonic::parse_in_normalized(Language::English, &normalised)
        .map_err(|_| WalletError::InvalidMnemonic)
}

pub fn to_seed(mnemonic: &Mnemonic) -> Zeroizing<[u8; SEED_LEN]> {
    Zeroizing::new(mnemonic.to_seed_normalized(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_phrases_parse_back() {
        let phrase = generate().unwrap();
        assert_eq!(phrase.split_whitespace().count(), NEW_WALLET_WORDS);
        assert!(parse(&phrase).is_ok());
    }

    #[test]
    fn tolerates_messy_whitespace() {
        let phrase = generate().unwrap();
        let messy = phrase.replace(' ', "\n  ");
        assert!(parse(&messy).is_ok());
    }

    const VALID: &str = "abandon abandon abandon abandon abandon abandon                          abandon abandon abandon abandon abandon about";

    #[test]
    fn accepts_a_known_good_phrase() {
        assert!(parse(VALID).is_ok());
    }

    #[test]
    fn rejects_a_bad_checksum() {

        let tampered = VALID.replace("about", "abandon");
        assert!(parse(&tampered).is_err());
        assert!(parse("zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo").is_err());
    }

    #[test]
    fn rejects_non_wordlist_input() {
        assert!(parse("not actually a seed phrase at all no").is_err());
    }
}
