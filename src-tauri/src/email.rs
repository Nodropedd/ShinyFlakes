//! Sending mail through the user's own SMTP server.
//!
//! This is the one outbound path that is not a chain lookup. It goes straight
//! to the mail provider the user configured, over TLS, with their app
//! password. No relay of ours is involved, matching the wallet's no-server
//! stance: the "server" is the user's own mailbox.

use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::appconfig::Smtp;
use crate::error::{Result, WalletError};

fn bad(e: impl std::fmt::Display) -> WalletError {
    WalletError::Network(format!("email: {e}"))
}

/// Sends one plain-text message to the configured mailbox.
///
/// STARTTLS on the given port; the connection is encrypted before the app
/// password is offered, so it never crosses the network in the clear.
pub async fn send(smtp: &Smtp, subject: &str, body: &str) -> Result<()> {
    let from = smtp
        .from
        .parse()
        .map_err(|_| bad("the From address is not valid"))?;
    let to = smtp
        .from
        .parse()
        .map_err(|_| bad("the destination address is not valid"))?;

    let message = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(bad)?;

    let creds = Credentials::new(smtp.username.clone(), smtp.password.clone());

    let mailer: AsyncSmtpTransport<Tokio1Executor> =
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
            .map_err(bad)?
            .port(smtp.port)
            .credentials(creds)
            .build();

    mailer.send(message).await.map_err(bad)?;
    Ok(())
}

/// The note sent when the seed or a private key is revealed.
pub fn reveal_body(what: &str) -> String {
    format!(
        "ShinyFlakes: your {what} was just revealed on your wallet's machine.\n\n\
         If this was you, no action is needed. If it was not, your device may be \
         in someone else's hands. Move your funds using your seed phrase on a \
         machine you trust, as soon as you can."
    )
}
