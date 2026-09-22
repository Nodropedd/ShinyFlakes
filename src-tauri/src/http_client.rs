//! Shared HTTP client.

pub fn builder() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();

    #[cfg(target_os = "android")]
    let builder = builder.tls_certs_only(android::roots());

    builder
}

#[cfg(target_os = "android")]
mod android {
    use std::sync::OnceLock;

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
