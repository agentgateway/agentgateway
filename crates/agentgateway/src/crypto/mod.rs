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

// A crypto backend must be selected at compile time. `crypto-aws-lc` is currently
// the only backend; as additional providers are added this becomes an
// exactly-one-of guard.
#[cfg(not(feature = "crypto-aws-lc"))]
compile_error!("no crypto backend selected: enable the `crypto-aws-lc` feature");

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
