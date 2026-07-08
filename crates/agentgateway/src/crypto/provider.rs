//! Construction of the rustls [`CryptoProvider`] for the compiled-in backend.
//!
//! This is the single place where the concrete crypto backend is selected. All
//! TLS configuration should obtain its provider from here (via [`provider`] or
//! [`provider_with_options`]) rather than referencing a backend crate directly.

use std::sync::Arc;

use rustls::crypto::CryptoProvider;

use crate::transport::tls::{
	CipherSuite, DEFAULT_CIPHER_SUITES, DEFAULT_KEY_EXCHANGE_GROUPS, KeyExchangeGroup,
};

/// Returns a [`CryptoProvider`] for the compiled-in backend using the default
/// cipher suites and key exchange groups.
pub fn provider() -> Arc<CryptoProvider> {
	provider_with_options(&[], &[])
}

/// Returns a [`CryptoProvider`] restricted to the given cipher suites and key
/// exchange groups. An empty slice means "use the backend defaults".
pub fn provider_with_options(
	cipher_suites: &[CipherSuite],
	key_exchange_groups: &[KeyExchangeGroup],
) -> Arc<CryptoProvider> {
	let cipher_suites = if cipher_suites.is_empty() {
		DEFAULT_CIPHER_SUITES.to_vec()
	} else {
		cipher_suites
			.iter()
			.map(CipherSuite::to_supported_cipher_suite)
			.collect()
	};

	let key_exchange_groups = if key_exchange_groups.is_empty() {
		DEFAULT_KEY_EXCHANGE_GROUPS.to_vec()
	} else {
		key_exchange_groups
			.iter()
			.map(KeyExchangeGroup::to_supported_kx_group)
			.collect()
	};

	let mut provider = default_crypto_provider();
	// Restrict negotiation to our allowlist.
	provider.cipher_suites = cipher_suites;
	provider.kx_groups = key_exchange_groups;
	Arc::new(provider)
}

/// Returns a [`CryptoProvider`] restricted to the given cipher suites, using the
/// default key exchange groups.
pub fn provider_with_cipher_suites(cipher_suites: &[CipherSuite]) -> Arc<CryptoProvider> {
	provider_with_options(cipher_suites, &[])
}

#[cfg(feature = "tls-aws-lc")]
fn default_crypto_provider() -> CryptoProvider {
	rustls::crypto::aws_lc_rs::default_provider()
}

#[cfg(feature = "tls-openssl")]
fn default_crypto_provider() -> CryptoProvider {
	rustls_openssl::default_provider()
}
