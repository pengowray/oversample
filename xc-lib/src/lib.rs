pub mod api;
pub mod cache;
pub mod group_words;
pub mod key_store;
pub mod metadata;
pub mod taxonomy;
pub mod types;

pub use types::*;

/// Build the HTTP client used for Xeno-Canto requests.
///
/// TLS uses rustls with the ring crypto provider, and certificates are checked
/// by the operating system's verifier (`rustls-platform-verifier`), so root
/// store changes and revocations arrive with OS updates instead of app
/// releases. On Android the verifier must be initialised from the app first;
/// see `src-tauri/src/tls.rs`.
#[cfg(feature = "http")]
pub fn http_client() -> reqwest::Client {
    // reqwest's `rustls-no-provider` feature needs a process-wide default.
    // Ignore the error: it only means a provider is already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}
