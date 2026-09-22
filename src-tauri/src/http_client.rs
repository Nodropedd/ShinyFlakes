//! The one place HTTP clients are made.
//!
//! reqwest 0.13 checks TLS certificates through the operating system, by way
//! of `rustls-platform-verifier`. On Android that crate has to be handed the
//! JVM and an application context before its first use, and nothing in this
//! app does that. So the first HTTPS request panicked with "Expect
//! rustls-platform-verifier to be initialized", and because the release
//! profile sets `panic = "abort"`, that panic took the whole app down the
//! moment the user pressed "Connect and load balances".
//!
//! On Android the client instead trusts Mozilla's root store, compiled in.
//! That needs no JVM, and it behaves the same on every Android release: 7.0,
//! the oldest this app supports, predates the root Let's Encrypt signs with,
//! so the phone's own store would turn away a good share of the services this
//! wallet talks to. The costs: a root the OS adds after a build is not trusted
//! until the next build, and a CA the user installed on the phone is never
//! trusted. For a wallet the second is the right answer anyway — a proxy
//! certificate on the device cannot read or rewrite its traffic.
//!
//! Every other platform keeps the OS verifier, which works there as it is.
//!
//! Anything that builds a `reqwest::Client` must start from [`builder`]. A
//! client built straight from `reqwest::Client::builder()` panics on Android
//! at its first HTTPS request.

/// A client builder with the TLS roots this platform needs.
pub fn builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();

    // `tls_certs_only` rather than `tls_certs_merge`: merging extra roots into
    // the platform verifier is exactly the path that is unavailable on
    // Android, and reqwest refuses it there.
    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(android::roots());

    builder
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::OnceLock;

    /// Mozilla's roots, converted once. A client is built for most requests,
    /// and there are well over a hundred of these.
    pub fn roots() -> Vec<reqwest::Certificate> {
        static ROOTS: OnceLock<Vec<reqwest::Certificate>> = OnceLock::new();
        ROOTS
            .get_or_init(|| {
                webpki_root_certs::TLS_SERVER_ROOT_CERTS
                    .iter()
                    .filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok())
                    .collect()
            })
            .clone()
    }
}
