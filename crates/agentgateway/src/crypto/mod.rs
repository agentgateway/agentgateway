//! Central cryptography module.
//!
//! Policy: all cryptographic operations in agentgateway SHOULD go through this
//! module so that the underlying crypto backend is pluggable and auditable. The
//! active backend is selected at compile time via `crypto-*` features (see
//! [`CRYPTO_BACKEND`]).
//!
//! Some operations cannot yet be routed through a pluggable backend (for
//! example certificate generation via `rcgen`, or legacy password hashing).
//! Such documented exceptions must be guarded with the appropriate
//! `#[cfg(feature = ...)]` so the backend in use stays explicit.

// Exactly one crypto backend must be selected at compile time.
#[cfg(not(any(feature = "crypto-aws-lc", feature = "crypto-symcrypt")))]
compile_error!(
	"no crypto backend selected: enable exactly one of `crypto-aws-lc` or `crypto-symcrypt`"
);

#[cfg(all(feature = "crypto-aws-lc", feature = "crypto-symcrypt"))]
compile_error!(
	"multiple crypto backends selected: enable exactly one of `crypto-aws-lc` or `crypto-symcrypt` (pass --no-default-features for a non-default backend)"
);

pub mod aead;
pub mod digest;
pub mod jwt;
pub mod rand;
pub mod tls;

pub use tls::{provider, provider_with_cipher_suites, provider_with_options};

/// Initializes process-global crypto state for the compiled-in backend.
///
/// Call once at startup, before any cryptographic operation that depends on an
/// installed provider (currently JWT signing/verification via [`jwt`]).
pub fn init() {
	jwt::init();
}

/// Human-readable name of the crypto backend compiled into this binary. Useful
/// for startup logging and diagnostics.
#[cfg(feature = "crypto-aws-lc")]
pub const CRYPTO_BACKEND: &str = "aws-lc-rs";

#[cfg(feature = "crypto-symcrypt")]
pub const CRYPTO_BACKEND: &str = "symcrypt";
