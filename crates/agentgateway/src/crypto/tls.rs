//! Construction of the rustls [`CryptoProvider`] for the compiled-in backend.
//!
//! This is the single place where the concrete crypto backend is selected. All
//! TLS configuration should obtain its provider from here (via [`provider`] or
//! [`provider_with_options_validated`]) rather than referencing a backend crate
//! directly.

// This is the one module allowed to assemble and customize rustls providers.
#![allow(clippy::disallowed_fields, clippy::disallowed_methods)]

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

pub(crate) fn key_provider(provider: &CryptoProvider) -> &'static dyn rustls::crypto::KeyProvider {
	provider.key_provider
}

pub(crate) fn signature_verification_algorithms() -> rustls::crypto::WebPkiSupportedAlgorithms {
	provider().signature_verification_algorithms
}

/// Returns a [`CryptoProvider`] restricted to the given cipher suites and key
/// exchange groups. An empty slice means "use the backend defaults".
pub(crate) fn provider_with_options(
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

/// Like [`provider_with_options`], but rejects selections the compiled-in backend
/// must not negotiate. Use this for anything derived from user configuration:
/// overriding the defaults otherwise bypasses the backend's own filtering and
/// yields a provider that silently reports `fips() == false`.
///
/// In a FIPS build a non-approved suite or group is an error, so the affected
/// listener or backend fails to build instead of degrading TLS at request time.
/// Outside a FIPS build every selection is permitted and this is equivalent to
/// [`provider_with_options`].
pub fn provider_with_options_validated(
	cipher_suites: &[CipherSuite],
	key_exchange_groups: &[KeyExchangeGroup],
) -> anyhow::Result<Arc<CryptoProvider>> {
	#[cfg(feature = "fips")]
	{
		// Ask the backend rather than maintaining a parallel table: the linked module
		// is the authority on what it treats as approved, and a hardcoded list would
		// drift from it.
		let bad_suites: Vec<&str> = cipher_suites
			.iter()
			.filter(|c| !c.to_supported_cipher_suite().fips())
			.map(|c| c.as_str_name())
			.collect();
		let bad_groups: Vec<&str> = key_exchange_groups
			.iter()
			.filter(|g| !g.to_supported_kx_group().fips())
			.map(|g| g.as_str_name())
			.collect();
		if !bad_suites.is_empty() || !bad_groups.is_empty() {
			anyhow::bail!(
				"TLS configuration is not permitted in a FIPS build (backend {}): \
				 non-approved cipher suites [{}], non-approved key exchange groups [{}]",
				crate::crypto::CRYPTO_BACKEND,
				bad_suites.join(", "),
				bad_groups.join(", "),
			);
		}
	}
	Ok(provider_with_options(cipher_suites, key_exchange_groups))
}

/// Verifies the default provider actually operates in FIPS mode, and fails closed
/// if not. Called from [`crate::crypto::init`] at startup: a build that links the
/// FIPS module but assembles a provider containing a non-approved suite or group is
/// not in FIPS mode, and must not serve traffic while claiming to be.
#[cfg(feature = "fips")]
pub fn assert_fips_provider() {
	panic_unless_fips(&provider());
}

/// Panics unless `provider` operates in FIPS mode, naming what disqualified it.
#[cfg(feature = "fips")]
fn panic_unless_fips(provider: &CryptoProvider) {
	if provider.fips() {
		return;
	}
	let suites: Vec<_> = provider
		.cipher_suites
		.iter()
		.filter(|c| !c.fips())
		.map(|c| format!("{:?}", c.suite()))
		.collect();
	let groups: Vec<_> = provider
		.kx_groups
		.iter()
		.filter(|g| !g.fips())
		.map(|g| format!("{:?}", g.name()))
		.collect();
	panic!(
		"crypto backend {} does not operate in FIPS mode; refusing to start. \
		 Non-approved cipher suites: [{}]. Non-approved key exchange groups: [{}].",
		crate::crypto::CRYPTO_BACKEND,
		suites.join(", "),
		groups.join(", "),
	);
}

#[cfg(feature = "crypto-aws-lc")]
fn default_crypto_provider() -> CryptoProvider {
	rustls::crypto::aws_lc_rs::default_provider()
}

#[cfg(feature = "crypto-symcrypt")]
fn default_crypto_provider() -> CryptoProvider {
	rustls_symcrypt::default_symcrypt_provider()
}

#[cfg(test)]
mod tests {
	use crate::transport::tls::{CipherSuite, KeyExchangeGroup};

	// Exercises the compiled-in backend's provider construction and the
	// cipher-suite / kx-group mappings (aws-lc-rs or SymCrypt).
	#[test]
	fn provider_has_default_suites_and_kx() {
		let p = super::provider();
		assert!(
			!p.cipher_suites.is_empty(),
			"default cipher suites must not be empty"
		);
		assert!(
			!p.kx_groups.is_empty(),
			"default kx groups must not be empty"
		);
	}

	// The FIPS build must actually select the FIPS module, not merely compile.
	#[cfg(feature = "fips")]
	#[test]
	fn provider_is_fips() {
		let p = super::provider();
		assert!(p.fips(), "the `fips` mode must yield a FIPS provider");
	}

	// Approved selections must be accepted by every backend.
	#[test]
	fn validated_provider_accepts_approved_selection() {
		let p = super::provider_with_options_validated(
			&[CipherSuite::TLS_AES_256_GCM_SHA384],
			&[KeyExchangeGroup::P256],
		)
		.expect("approved selection must be accepted");
		assert_eq!(p.cipher_suites.len(), 1);
		assert_eq!(p.kx_groups.len(), 1);
	}

	// In a FIPS build, a non-approved selection must be refused at config time
	// rather than producing a provider that reports fips() == false.
	#[cfg(feature = "fips")]
	#[test]
	fn validated_provider_rejects_non_approved_selection() {
		let err =
			super::provider_with_options_validated(&[CipherSuite::TLS_CHACHA20_POLY1305_SHA256], &[])
				.expect_err("ChaCha20 must be rejected in a FIPS build");
		assert!(
			err.to_string().contains("TLS_CHACHA20_POLY1305_SHA256"),
			"error should name the offending suite, got: {err}"
		);

		let err = super::provider_with_options_validated(&[], &[KeyExchangeGroup::X25519])
			.expect_err("bare X25519 must be rejected in a FIPS build");
		assert!(
			err.to_string().contains("X25519"),
			"error should name the offending group, got: {err}"
		);
	}

	// Outside a FIPS build the same selections stay available.
	#[cfg(not(feature = "fips"))]
	#[test]
	fn validated_provider_permits_everything_outside_fips() {
		super::provider_with_options_validated(
			&[CipherSuite::TLS_CHACHA20_POLY1305_SHA256],
			&[KeyExchangeGroup::X25519],
		)
		.expect("non-FIPS builds must not restrict selections");
	}

	// TLS 1.2 is permitted under FIPS 140-3 provided Extended Master Secret is
	// required. rustls derives `require_ems` from `provider.fips()`, so a config
	// built on the FIPS provider must report fips() even when TLS 1.2 is offered.
	// This is the check that would fail first if we ever needed to drop to 1.3-only.
	#[cfg(feature = "fips")]
	#[test]
	fn tls12_config_is_fips_via_ems() {
		use rustls::{ClientConfig, RootCertStore};
		let cfg = ClientConfig::builder_with_provider(super::provider())
			.with_protocol_versions(&[&rustls::version::TLS12])
			.expect("TLS 1.2 must remain usable in a FIPS build")
			.with_root_certificates(RootCertStore::empty())
			.with_no_client_auth();
		assert!(
			cfg.fips(),
			"TLS 1.2 config must satisfy FIPS via Extended Master Secret"
		);
	}

	#[test]
	fn provider_with_options_applies_selection() {
		let p = super::provider_with_options(
			&[CipherSuite::TLS_AES_256_GCM_SHA384],
			&[KeyExchangeGroup::P256],
		);
		assert_eq!(p.cipher_suites.len(), 1);
		assert_eq!(p.kx_groups.len(), 1);
	}
}
