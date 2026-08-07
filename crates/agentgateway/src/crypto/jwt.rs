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
	#[cfg(feature = "crypto-aws-lc")]
	{
		// aws-lc-rs is already jsonwebtoken's default; installing explicitly keeps
		// the choice in this seam. A backend with no built-in provider (e.g.
		// SymCrypt) would install a custom one here instead.
		let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
	}
}
