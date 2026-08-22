//! JWT crypto seam.
//!
//! JWT crypto lives inside the `jsonwebtoken` crate, which routes its
//! `encode`/`decode` through its own process-global provider. Rather than
//! wrapping those calls, this module just selects which provider is active for
//! the compiled-in `crypto-*` backend, via [`init`].

/// Installs the process-global JWT crypto provider for the compiled-in backend.
///
/// Call once at startup, before any JWT signing or verification. Idempotent.
pub fn init() {
	// JWT always uses aws-lc-rs: SymCrypt has no jsonwebtoken provider, so
	// `crypto-symcrypt` falls back to aws-lc-rs here.
	#[cfg(any(feature = "crypto-aws-lc", feature = "crypto-symcrypt"))]
	{
		let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
	}
}
