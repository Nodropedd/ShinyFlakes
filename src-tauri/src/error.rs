use serde::ser::SerializeStruct;

/// Every failure the UI can see. Messages are written for a person reading a
/// dialog, and deliberately say nothing about key material or file contents.
#[derive(Debug, thiserror::Error)]
pub enum WalletError {
    #[error("That is not a valid BIP-39 seed phrase.")]
    InvalidMnemonic,

    #[error("That seed phrase does not match this wallet.")]
    WrongSeed,

    #[error("The wallet is locked.")]
    Locked,

    #[error("A wallet already exists on this machine.")]
    VaultExists,

    #[error("No wallet exists on this machine yet.")]
    NoVault,

    #[error("The OS keychain could not be reached: {0}")]
    Keychain(String),

    #[error("The local vault could not be read or written: {0}")]
    Storage(String),

    #[error("The vault could not be decrypted. The file may be damaged.")]
    Decrypt,

    #[error("An address could not be derived from the seed: {0}")]
    Derivation(String),

    #[error("{0}")]
    Network(String),

    #[error("{0}")]
    Unsupported(String),

    #[error("{0}")]
    Funds(String),

    #[error("Two-factor confirmation is required first.")]
    TwoFactorRequired,

    #[error(
        "This machine has a wallet file but no longer holds the key that          decrypts it. The key lives in the Windows credential store and it          is gone. Restore from your seed phrase on a fresh install."
    )]
    KeyMissing,
}

impl WalletError {
    /// Stable machine-readable tag. The UI branches on this, never on the
    /// message text.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::InvalidMnemonic => "InvalidMnemonic",
            Self::WrongSeed => "WrongSeed",
            Self::Locked => "Locked",
            Self::VaultExists => "VaultExists",
            Self::NoVault => "NoVault",
            Self::Keychain(_) => "Keychain",
            Self::Storage(_) => "Storage",
            Self::Decrypt => "Decrypt",
            Self::Derivation(_) => "Derivation",
            Self::Network(_) => "Network",
            Self::Unsupported(_) => "Unsupported",
            Self::Funds(_) => "Funds",
            Self::TwoFactorRequired => "TwoFactorRequired",
            Self::KeyMissing => "KeyMissing",
        }
    }
}

impl serde::Serialize for WalletError {
    // Fully qualified: the Result alias below shadows the std one in this
    // module, and the trait requires the serializer's own error type.
    fn serialize<S: serde::Serializer>(
        &self,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("IpcError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

pub type Result<T> = std::result::Result<T, WalletError>;
