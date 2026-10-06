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

#[cfg(feature = "fips")]
mod fips {
	use std::sync::{LazyLock, Once};

	use jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER;
	use jsonwebtoken::crypto::{CryptoProvider, JwtSigner, JwtVerifier};
	use jsonwebtoken::errors::{ErrorKind, Result};
	use jsonwebtoken::{Algorithm, AlgorithmFamily, DecodingKey, DecodingKeyKind, EncodingKey};

	const MIN_HMAC_KEY_BYTES: usize = 14;

	pub(super) static PROVIDER: LazyLock<CryptoProvider> = LazyLock::new(|| CryptoProvider {
		signer_factory,
		verifier_factory,
		key_utils: DEFAULT_PROVIDER.key_utils.clone(),
	});

	pub(super) fn install() {
		// Repeated initialization must not try to install our provider again.
		static INSTALL: Once = Once::new();
		INSTALL.call_once(|| {
			assert!(
				PROVIDER.install_default().is_ok(),
				"a JWT crypto provider was installed before crypto::init(); refusing to run without the FIPS policy"
			);
		});
	}

	fn signer_factory(alg: &Algorithm, key: &EncodingKey) -> Result<Box<dyn JwtSigner>> {
		match key.family() {
			AlgorithmFamily::Rsa => {
				let (n, e) = (DEFAULT_PROVIDER
					.key_utils
					.rsa_pub_components_from_private_key)(key.as_bytes())?;
				check_rsa(&n, &e)?;
			},
			AlgorithmFamily::Hmac => check_hmac(key.as_bytes())?,
			_ => {},
		}
		(DEFAULT_PROVIDER.signer_factory)(alg, key)
	}

	fn verifier_factory(alg: &Algorithm, key: &DecodingKey) -> Result<Box<dyn JwtVerifier>> {
		match (key.family(), key.kind()) {
			(AlgorithmFamily::Rsa, DecodingKeyKind::RsaModulusExponent { n, e }) => check_rsa(n, e)?,
			(AlgorithmFamily::Rsa, DecodingKeyKind::SecretOrDer(der)) => {
				let (n, e) = (DEFAULT_PROVIDER
					.key_utils
					.rsa_pub_components_from_public_key)(der)?;
				check_rsa(&n, &e)?;
			},
			(AlgorithmFamily::Hmac, DecodingKeyKind::SecretOrDer(secret)) => check_hmac(secret)?,
			_ => {},
		}
		(DEFAULT_PROVIDER.verifier_factory)(alg, key)
	}

	/// Checks the FIPS 186-5 RSA rules the backend doesn't: an even modulus size
	/// and a public exponent above 2^16. `n` and `e` are big-endian.
	fn check_rsa(n: &[u8], e: &[u8]) -> Result<()> {
		let bits = bit_len(n);
		if !bits.is_multiple_of(2) {
			return Err(
				ErrorKind::InvalidRsaKey(format!(
					"{bits}-bit modulus is not permitted in FIPS mode (even size required)"
				))
				.into(),
			);
		}
		let exponent_above_minimum = match bit_len(e) {
			0..=16 => false,
			// With 17 significant bits, only 0x01_00_00 is too small.
			17 => !e.ends_with(&[0, 0]),
			_ => true,
		};
		if !exponent_above_minimum {
			return Err(
				ErrorKind::InvalidRsaKey(
					"public exponent is not permitted in FIPS mode (must be greater than 2^16)".to_string(),
				)
				.into(),
			);
		}
		Ok(())
	}

	/// Rejects HMAC keys shorter than 112 bits (SP 800-131A).
	fn check_hmac(secret: &[u8]) -> Result<()> {
		if secret.len() < MIN_HMAC_KEY_BYTES {
			return Err(
				ErrorKind::Provider(format!(
					"{}-bit HMAC key is not permitted in FIPS mode (at least {} bits required)",
					secret.len() * 8,
					MIN_HMAC_KEY_BYTES * 8
				))
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

	#[cfg(test)]
	mod tests {
		use rstest::rstest;

		use super::*;

		#[rstest]
		#[case::odd_below(2047, false)]
		#[case::even(2048, true)]
		#[case::odd_above(2049, false)]
		#[case::even_above(2050, true)]
		fn rsa_modulus_parity(#[case] bits: usize, #[case] allowed: bool) {
			let result = check_rsa(&modulus(bits), &[1, 0, 1]);
			assert_eq!(result.is_ok(), allowed);
		}

		#[rstest]
		#[case::empty(&[], false)]
		#[case::zero(&[0], false)]
		#[case::three(&[3], false)]
		#[case::below_boundary(&[0xff, 0xff], false)]
		#[case::at_boundary(&[1, 0, 0], false)]
		#[case::above_boundary(&[1, 0, 1], true)]
		#[case::larger_exponent(&[4, 0, 1], true)]
		#[case::boundary_with_leading_zero(&[0, 1, 0, 0], false)]
		#[case::above_boundary_with_leading_zero(&[0, 1, 0, 1], true)]
		fn rsa_exponent_lower_bound(#[case] exponent: &[u8], #[case] allowed: bool) {
			let result = check_rsa(&modulus(2048), exponent);
			assert_eq!(result.is_ok(), allowed);
		}

		#[rstest]
		#[case::rs256(Algorithm::RS256)]
		#[case::rs384(Algorithm::RS384)]
		#[case::rs512(Algorithm::RS512)]
		#[case::ps256(Algorithm::PS256)]
		#[case::ps384(Algorithm::PS384)]
		#[case::ps512(Algorithm::PS512)]
		fn rsa_provider_signs_and_verifies(#[case] algorithm: Algorithm) {
			let signing_key = EncodingKey::from_rsa_pem(include_bytes!("testdata/rsa2048.pem")).unwrap();
			let verification_key = DecodingKey::from_rsa_der(include_bytes!("testdata/rsa2048.pub.der"));
			assert_signs_and_verifies(algorithm, &signing_key, &verification_key);
		}

		#[rstest]
		#[case::es256(Algorithm::ES256, &rcgen::PKCS_ECDSA_P256_SHA256)]
		#[case::es384(Algorithm::ES384, &rcgen::PKCS_ECDSA_P384_SHA384)]
		fn ecdsa_provider_signs_and_verifies(
			#[case] algorithm: Algorithm,
			#[case] key_alg: &'static rcgen::SignatureAlgorithm,
		) {
			let key_pair = rcgen::KeyPair::generate_for(key_alg).unwrap();
			let signing_key = EncodingKey::from_ec_pem(key_pair.serialize_pem().as_bytes()).unwrap();
			let verification_key =
				DecodingKey::from_ec_pem(key_pair.public_key_pem().as_bytes()).unwrap();
			assert_signs_and_verifies(algorithm, &signing_key, &verification_key);
		}

		#[test]
		fn eddsa_provider_signs_and_verifies() {
			let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
			let signing_key = EncodingKey::from_ed_pem(key_pair.serialize_pem().as_bytes()).unwrap();
			let verification_key =
				DecodingKey::from_ed_pem(key_pair.public_key_pem().as_bytes()).unwrap();
			assert_signs_and_verifies(Algorithm::EdDSA, &signing_key, &verification_key);
		}

		#[rstest]
		#[case::hs256(Algorithm::HS256)]
		#[case::hs384(Algorithm::HS384)]
		#[case::hs512(Algorithm::HS512)]
		fn hmac_provider_signs_and_verifies(#[case] algorithm: Algorithm) {
			let secret = [7u8; 32];
			assert_signs_and_verifies(
				algorithm,
				&EncodingKey::from_secret(&secret),
				&DecodingKey::from_secret(&secret),
			);
		}

		#[rstest]
		#[case::empty(0, false)]
		#[case::below_minimum(13, false)]
		#[case::minimum(14, true)]
		fn hmac_key_length_policy(#[case] len: usize, #[case] allowed: bool) {
			let secret = vec![7u8; len];
			let signer = (PROVIDER.signer_factory)(&Algorithm::HS256, &EncodingKey::from_secret(&secret));
			let verifier =
				(PROVIDER.verifier_factory)(&Algorithm::HS256, &DecodingKey::from_secret(&secret));
			assert_eq!(signer.is_ok(), allowed);
			assert_eq!(verifier.is_ok(), allowed);
		}

		#[test]
		fn provider_rejects_non_approved_rsa_keys() {
			let is_invalid_rsa = |kind: &ErrorKind| matches!(kind, ErrorKind::InvalidRsaKey(_));

			let signing_key = EncodingKey::from_rsa_pem(include_bytes!("testdata/rsa2049.pem")).unwrap();
			let err = (PROVIDER.signer_factory)(&Algorithm::RS256, &signing_key)
				.err()
				.unwrap();
			assert!(is_invalid_rsa(err.kind()), "{err:?}");

			for (name, key) in [
				(
					"der",
					DecodingKey::from_rsa_der(include_bytes!("testdata/rsa2049.pub.der")),
				),
				(
					"components",
					DecodingKey::from_rsa_raw_components(&modulus(2049), &[1, 0, 1]),
				),
			] {
				let err = (PROVIDER.verifier_factory)(&Algorithm::RS256, &key)
					.err()
					.unwrap();
				assert!(is_invalid_rsa(err.kind()), "{name}: {err:?}");
			}
		}

		fn assert_signs_and_verifies(
			algorithm: Algorithm,
			signing_key: &EncodingKey,
			verification_key: &DecodingKey,
		) {
			let signer = (PROVIDER.signer_factory)(&algorithm, signing_key).unwrap();
			let verifier = (PROVIDER.verifier_factory)(&algorithm, verification_key).unwrap();

			let signature = signer.try_sign(b"test message").unwrap();

			verifier.verify(b"test message", &signature).unwrap();
			assert!(verifier.verify(b"modified message", &signature).is_err());
		}

		fn modulus(bits: usize) -> Vec<u8> {
			let mut n = vec![0xff; bits.div_ceil(8)];
			let unused_bits = n.len() * 8 - bits;
			n[0] >>= unused_bits;
			n
		}
	}
}
