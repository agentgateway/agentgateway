//! Central cryptography module.
//!
//! Policy: all cryptographic operations in agentgateway SHOULD go through this
//! module so that the underlying crypto backend is pluggable and auditable. The
//! active backend is selected at compile time via mutually-exclusive `tls-*`
//! features (see [`CRYPTO_BACKEND`]).
//!
//! Some operations cannot yet be routed through a pluggable backend (for
//! example certificate generation via `rcgen`, or legacy password hashing).
//! Such documented exceptions must be guarded with the appropriate
//! `#[cfg(feature = ...)]` so the backend in use stays explicit.

// Exactly one crypto backend must be selected at compile time.
#[cfg(not(any(feature = "tls-aws-lc", feature = "tls-openssl")))]
compile_error!(
	"no crypto backend selected: enable exactly one of the `tls-aws-lc` or `tls-openssl` features"
);

#[cfg(all(feature = "tls-aws-lc", feature = "tls-openssl"))]
compile_error!(
	"multiple crypto backends selected: enable exactly one of the `tls-aws-lc` or `tls-openssl` features"
);

pub mod aead;
pub mod digest;
pub mod provider;
pub mod rand;

pub use provider::{provider, provider_with_cipher_suites, provider_with_options};

/// Human-readable name of the crypto backend compiled into this binary. Useful
/// for startup logging and diagnostics.
#[cfg(feature = "tls-aws-lc")]
pub const CRYPTO_BACKEND: &str = "aws-lc-rs";

#[cfg(feature = "tls-openssl")]
pub const CRYPTO_BACKEND: &str = "openssl";
