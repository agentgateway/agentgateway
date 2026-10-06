//! JWT crypto seam.
//!
//! JWT crypto lives inside the `jsonwebtoken` crate, which routes its
//! `encode`/`decode` through its own process-global provider. Rather than
//! wrapping those calls, this module just selects which provider is active for
//! the compiled-in `crypto-*` backend, via [`init`].

/// Installs the process-global JWT crypto provider for the compiled-in backend.
///
/// Call once at startup, before any JWT signing or verification: `jsonwebtoken`
/// otherwise latches its own default provider on first use. Idempotent.
///
/// In a FIPS build, panics if a different provider is already active, since
/// JWT operations would then bypass the FIPS policy.
pub fn init() {
	// JWT always uses aws-lc-rs: SymCrypt has no jsonwebtoken provider, so
	// `crypto-symcrypt` falls back to aws-lc-rs here.
	#[cfg(all(
		any(feature = "crypto-aws-lc", feature = "crypto-symcrypt"),
		not(feature = "fips")
	))]
	{
		let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
	}
	#[cfg(feature = "fips")]
	fips::install();
}

/// Reports whether `alg` may be used for JWT signing or verification.
///
/// A FIPS build permits only approved asymmetric signatures: RSA PKCS#1 v1.5,
/// RSA-PSS and ECDSA. EdDSA is excluded until the linked module's certificate
/// is confirmed to cover Ed25519, and HMAC is conservatively excluded until a
/// FIPS-compliant secret-length policy is implemented. RSA key parameters are
/// checked by the FIPS provider; the backend itself limits RSA to 2048-8192 bits
/// and ECDSA to P-256 and P-384.
#[cfg(any(feature = "fips", test))]
fn algorithm_allowed(alg: jsonwebtoken::Algorithm) -> bool {
	#[cfg(feature = "fips")]
	{
		use jsonwebtoken::Algorithm;
		matches!(
			alg,
			Algorithm::RS256
				| Algorithm::RS384
				| Algorithm::RS512
				| Algorithm::PS256
				| Algorithm::PS384
				| Algorithm::PS512
				| Algorithm::ES256
				| Algorithm::ES384
		)
	}
	#[cfg(not(feature = "fips"))]
	{
		let _ = alg;
		true
	}
}

/// The aws-lc-rs provider restricted to [`algorithm_allowed`] and FIPS 186-5 RSA
/// key parameters, so every `jsonwebtoken` sign and verify call is subject to the
/// FIPS policy.
#[cfg(feature = "fips")]
mod fips {
	use std::sync::{LazyLock, Once};

	use jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER;
	use jsonwebtoken::crypto::{CryptoProvider, JwtSigner, JwtVerifier};
	use jsonwebtoken::errors::{ErrorKind, Result};
	use jsonwebtoken::{Algorithm, AlgorithmFamily, DecodingKey, DecodingKeyKind, EncodingKey};

	use super::algorithm_allowed;

	const MIN_RSA_BITS: usize = 2048;

	pub(super) static PROVIDER: LazyLock<CryptoProvider> = LazyLock::new(|| CryptoProvider {
		signer_factory,
		verifier_factory,
		key_utils: DEFAULT_PROVIDER.key_utils.clone(),
	});

	/// Installs [`PROVIDER`] once, panicking if another provider is already active.
	pub(super) fn install() {
		static INSTALL: Once = Once::new();
		INSTALL.call_once(|| {
			if PROVIDER.install_default().is_err() {
				panic!(
					"a JWT crypto provider was installed before crypto::init(); refusing to run without the FIPS policy"
				);
			}
		});
	}

	fn signer_factory(alg: &Algorithm, key: &EncodingKey) -> Result<Box<dyn JwtSigner>> {
		if !algorithm_allowed(*alg) {
			return Err(ErrorKind::InvalidAlgorithm.into());
		}
		if key.family() == AlgorithmFamily::Rsa {
			let (n, e) = (DEFAULT_PROVIDER
				.key_utils
				.rsa_pub_components_from_private_key)(key.as_bytes())?;
			check_rsa(&n, &e)?;
		}
		(DEFAULT_PROVIDER.signer_factory)(alg, key)
	}

	fn verifier_factory(alg: &Algorithm, key: &DecodingKey) -> Result<Box<dyn JwtVerifier>> {
		if !algorithm_allowed(*alg) {
			return Err(ErrorKind::InvalidAlgorithm.into());
		}
		if key.family() == AlgorithmFamily::Rsa {
			match key.kind() {
				DecodingKeyKind::RsaModulusExponent { n, e } => check_rsa(n, e)?,
				DecodingKeyKind::SecretOrDer(der) => {
					let (n, e) = (DEFAULT_PROVIDER
						.key_utils
						.rsa_pub_components_from_public_key)(der)?;
					check_rsa(&n, &e)?;
				},
			}
		}
		(DEFAULT_PROVIDER.verifier_factory)(alg, key)
	}

	/// Checks the FIPS 186-5 RSA conditions: an even modulus size of at least
	/// 2048 bits and a public exponent greater than 2^16. `n` and `e` are
	/// big-endian.
	pub(super) fn check_rsa(n: &[u8], e: &[u8]) -> Result<()> {
		let bits = bit_len(n);
		if bits < MIN_RSA_BITS || !bits.is_multiple_of(2) {
			return Err(
				ErrorKind::InvalidRsaKey(format!(
					"{bits}-bit modulus is not permitted in FIPS mode (even size of at least {MIN_RSA_BITS} bits required)"
				))
				.into(),
			);
		}
		// e > 2^16: more than 17 bits, or 17 bits other than 2^16 itself.
		let e_bits = bit_len(e);
		if e_bits < 17 || (e_bits == 17 && e.iter().rev().take(2).all(|b| *b == 0)) {
			return Err(
				ErrorKind::InvalidRsaKey(
					"public exponent is not permitted in FIPS mode (must be greater than 2^16)".to_string(),
				)
				.into(),
			);
		}
		Ok(())
	}

	fn bit_len(be: &[u8]) -> usize {
		match be.iter().position(|b| *b != 0) {
			Some(i) => (be.len() - i) * 8 - be[i].leading_zeros() as usize,
			None => 0,
		}
	}
}

#[cfg(test)]
mod tests {
	use jsonwebtoken::Algorithm;

	use super::*;

	#[test]
	fn asymmetric_algorithms_allowed() {
		for alg in [
			Algorithm::RS256,
			Algorithm::RS384,
			Algorithm::RS512,
			Algorithm::PS256,
			Algorithm::PS384,
			Algorithm::PS512,
			Algorithm::ES256,
			Algorithm::ES384,
		] {
			assert!(algorithm_allowed(alg), "{alg:?}");
		}
	}

	#[cfg(feature = "fips")]
	#[test]
	fn fips_rejects_non_approved_algorithms() {
		for alg in [
			Algorithm::EdDSA,
			Algorithm::HS256,
			Algorithm::HS384,
			Algorithm::HS512,
		] {
			assert!(!algorithm_allowed(alg), "{alg:?}");
		}
	}

	#[cfg(feature = "fips")]
	#[test]
	fn fips_provider_rejects_non_approved_algorithms() {
		use jsonwebtoken::errors::ErrorKind;
		use jsonwebtoken::{DecodingKey, EncodingKey};

		let provider = &*fips::PROVIDER;
		let secret = [7u8; 32];
		let ed_public = [0u8; 32];
		for (alg, key) in [
			(Algorithm::HS256, DecodingKey::from_secret(&secret)),
			(Algorithm::EdDSA, DecodingKey::from_ed_der(&ed_public)),
		] {
			let err = (provider.verifier_factory)(&alg, &key).err().unwrap();
			assert_eq!(err.kind(), &ErrorKind::InvalidAlgorithm, "{alg:?}");
		}
		let err = (provider.signer_factory)(&Algorithm::HS256, &EncodingKey::from_secret(&secret))
			.err()
			.unwrap();
		assert_eq!(err.kind(), &ErrorKind::InvalidAlgorithm);
	}

	#[cfg(feature = "fips")]
	#[test]
	fn fips_provider_allows_approved_keys() {
		use jsonwebtoken::{DecodingKey, EncodingKey};

		let provider = &*fips::PROVIDER;
		// x/y are the P-256 public key from the http::jwt test fixtures.
		let ec = DecodingKey::from_ec_components(
			"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk",
			"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug",
		)
		.unwrap();
		assert!((provider.verifier_factory)(&Algorithm::ES256, &ec).is_ok());

		let rsa_der = DecodingKey::from_rsa_der(include_bytes!("testdata/rsa2048.pub.der"));
		assert!((provider.verifier_factory)(&Algorithm::RS256, &rsa_der).is_ok());
		let rsa_components = DecodingKey::from_rsa_raw_components(&modulus(2048), &[1, 0, 1]);
		assert!((provider.verifier_factory)(&Algorithm::PS256, &rsa_components).is_ok());
		let rsa_signing = EncodingKey::from_rsa_pem(include_bytes!("testdata/rsa2048.pem")).unwrap();
		assert!((provider.signer_factory)(&Algorithm::RS256, &rsa_signing).is_ok());
	}

	#[cfg(feature = "fips")]
	#[test]
	fn fips_provider_rejects_non_approved_rsa_keys() {
		use jsonwebtoken::errors::ErrorKind;
		use jsonwebtoken::{DecodingKey, EncodingKey};

		let provider = &*fips::PROVIDER;
		let is_invalid_rsa = |kind: &ErrorKind| matches!(kind, ErrorKind::InvalidRsaKey(_));

		let odd_signing = EncodingKey::from_rsa_pem(include_bytes!("testdata/rsa2049.pem")).unwrap();
		let err = (provider.signer_factory)(&Algorithm::RS256, &odd_signing)
			.err()
			.unwrap();
		assert!(is_invalid_rsa(err.kind()), "{err:?}");

		let odd_der = DecodingKey::from_rsa_der(include_bytes!("testdata/rsa2049.pub.der"));
		for (name, key) in [
			("2049-bit der", odd_der),
			(
				"2049-bit components",
				DecodingKey::from_rsa_raw_components(&modulus(2049), &[1, 0, 1]),
			),
			(
				"1024-bit",
				DecodingKey::from_rsa_raw_components(&modulus(1024), &[1, 0, 1]),
			),
			(
				"exponent 3",
				DecodingKey::from_rsa_raw_components(&modulus(2048), &[3]),
			),
			(
				"exponent 2^16",
				DecodingKey::from_rsa_raw_components(&modulus(2048), &[1, 0, 0]),
			),
		] {
			let err = (provider.verifier_factory)(&Algorithm::RS256, &key)
				.err()
				.unwrap();
			assert!(is_invalid_rsa(err.kind()), "{name}: {err:?}");
		}
	}

	/// A big-endian modulus of exactly `bits` bits.
	#[cfg(feature = "fips")]
	fn modulus(bits: usize) -> Vec<u8> {
		let mut n = vec![0xffu8; bits.div_ceil(8)];
		n[0] = 0xff >> (n.len() * 8 - bits);
		n
	}

	#[cfg(not(feature = "fips"))]
	#[test]
	fn non_fips_allows_all_algorithms() {
		assert!(algorithm_allowed(Algorithm::EdDSA));
		assert!(algorithm_allowed(Algorithm::HS256));
	}
}
