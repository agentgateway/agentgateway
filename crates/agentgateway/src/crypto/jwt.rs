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

	const MIN_RSA_BITS: usize = 2048;

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

	/// Permits only approved asymmetric signatures: RSA PKCS#1 v1.5, RSA-PSS and
	/// ECDSA. EdDSA is excluded until the linked module's certificate is confirmed
	/// to cover Ed25519. The backend itself limits RSA to 2048-8192 bits and ECDSA
	/// to P-256 and P-384.
	fn algorithm_allowed(alg: Algorithm) -> bool {
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
	fn check_rsa(n: &[u8], e: &[u8]) -> Result<()> {
		let bits = bit_len(n);
		if bits < MIN_RSA_BITS || !bits.is_multiple_of(2) {
			return Err(
				ErrorKind::InvalidRsaKey(format!(
					"{bits}-bit modulus is not permitted in FIPS mode (even size of at least {MIN_RSA_BITS} bits required)"
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
		#[case::too_small(1024, false)]
		#[case::below_minimum(2047, false)]
		#[case::minimum(2048, true)]
		#[case::odd_size(2049, false)]
		#[case::even_size(2050, true)]
		fn rsa_modulus_size_policy(#[case] bits: usize, #[case] allowed: bool) {
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

		#[rstest]
		#[case::rs256(Algorithm::RS256, true)]
		#[case::rs384(Algorithm::RS384, true)]
		#[case::rs512(Algorithm::RS512, true)]
		#[case::ps256(Algorithm::PS256, true)]
		#[case::ps384(Algorithm::PS384, true)]
		#[case::ps512(Algorithm::PS512, true)]
		#[case::es256(Algorithm::ES256, true)]
		#[case::es384(Algorithm::ES384, true)]
		#[case::eddsa(Algorithm::EdDSA, false)]
		#[case::hs256(Algorithm::HS256, false)]
		#[case::hs384(Algorithm::HS384, false)]
		#[case::hs512(Algorithm::HS512, false)]
		fn algorithm_policy(#[case] algorithm: Algorithm, #[case] allowed: bool) {
			assert_eq!(algorithm_allowed(algorithm), allowed);
		}

		#[test]
		fn provider_rejects_non_approved_algorithms() {
			let secret = [7u8; 32];
			let ed_public = [0u8; 32];
			for (alg, key) in [
				(Algorithm::HS256, DecodingKey::from_secret(&secret)),
				(Algorithm::EdDSA, DecodingKey::from_ed_der(&ed_public)),
			] {
				let err = (PROVIDER.verifier_factory)(&alg, &key).err().unwrap();
				assert_eq!(err.kind(), &ErrorKind::InvalidAlgorithm, "{alg:?}");
			}
			let err = (PROVIDER.signer_factory)(&Algorithm::HS256, &EncodingKey::from_secret(&secret))
				.err()
				.unwrap();
			assert_eq!(err.kind(), &ErrorKind::InvalidAlgorithm);
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
