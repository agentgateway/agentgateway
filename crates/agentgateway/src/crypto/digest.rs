//! Cryptographic hash functions.
//!
//! This is the seam for hashing within the `agentgateway` crate. The backend is
//! selected by the `crypto-*` feature; `crypto-aws-lc` (the default) backs it
//! with `aws-lc-rs`. Additional backends plug in here behind `#[cfg]` without
//! changing call sites.
//!
//! Note: hashing in sibling crates (`agent-celx`, `htpasswd-verify-fork`) still
//! uses the RustCrypto crates directly, as they cannot depend on this crate.
//! Consolidating those requires a shared crypto crate and is tracked separately.

/// Length in bytes of a SHA-256 digest.
pub const SHA256_LEN: usize = 32;

/// Length in bytes of a SHA-512 digest.
pub const SHA512_LEN: usize = 64;

pub use imp::{Sha256, sha256, sha512};

#[cfg(feature = "crypto-aws-lc")]
mod imp {
	use aws_lc_rs::digest::{self, Context, SHA256, SHA512};

	use super::{SHA256_LEN, SHA512_LEN};

	/// Computes the SHA-256 digest of `data` in one shot.
	pub fn sha256(data: &[u8]) -> [u8; SHA256_LEN] {
		to_array(digest::digest(&SHA256, data).as_ref())
	}

	/// Computes the SHA-512 digest of `data` in one shot.
	pub fn sha512(data: &[u8]) -> [u8; SHA512_LEN] {
		to_array(digest::digest(&SHA512, data).as_ref())
	}

	/// Incremental SHA-256 hasher, for data supplied in multiple pieces.
	pub struct Sha256(Context);

	impl Sha256 {
		/// Creates a new, empty SHA-256 hasher.
		pub fn new() -> Self {
			Self(Context::new(&SHA256))
		}

		/// Adds `data` to the running digest.
		pub fn update(&mut self, data: impl AsRef<[u8]>) {
			self.0.update(data.as_ref());
		}

		/// Consumes the hasher and returns the final digest.
		pub fn finalize(self) -> [u8; SHA256_LEN] {
			to_array(self.0.finish().as_ref())
		}
	}

	impl Default for Sha256 {
		fn default() -> Self {
			Self::new()
		}
	}

	fn to_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
		let mut out = [0u8; N];
		out.copy_from_slice(bytes);
		out
	}
}

#[cfg(feature = "crypto-symcrypt")]
mod imp {
	use super::{SHA256_LEN, SHA512_LEN};

	/// Computes the SHA-256 digest of `data` in one shot (SymCrypt backend).
	pub fn sha256(data: &[u8]) -> [u8; SHA256_LEN] {
		symcrypt::hash::sha256(data)
	}

	/// Computes the SHA-512 digest of `data` in one shot (SymCrypt backend).
	pub fn sha512(data: &[u8]) -> [u8; SHA512_LEN] {
		symcrypt::hash::sha512(data)
	}

	/// Incremental SHA-256 hasher, for data supplied in multiple pieces.
	pub struct Sha256(symcrypt::hash::Sha256State);

	impl Sha256 {
		/// Creates a new, empty SHA-256 hasher.
		pub fn new() -> Self {
			Self(symcrypt::hash::Sha256State::new())
		}

		/// Adds `data` to the running digest.
		pub fn update(&mut self, data: impl AsRef<[u8]>) {
			use symcrypt::hash::HashState;
			self.0.append(data.as_ref());
		}

		/// Consumes the hasher and returns the final digest.
		pub fn finalize(mut self) -> [u8; SHA256_LEN] {
			use symcrypt::hash::HashState;
			self.0.result()
		}
	}

	impl Default for Sha256 {
		fn default() -> Self {
			Self::new()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{Sha256, sha256, sha512};

	// SHA-256("abc") known-answer vector (FIPS 180-4).
	const ABC: [u8; 32] = [
		0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
		0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
	];

	// SHA-512("abc") known-answer vector (FIPS 180-4).
	const ABC_512: [u8; 64] = [
		0xdd, 0xaf, 0x35, 0xa1, 0x93, 0x61, 0x7a, 0xba, 0xcc, 0x41, 0x73, 0x49, 0xae, 0x20, 0x41, 0x31,
		0x12, 0xe6, 0xfa, 0x4e, 0x89, 0xa9, 0x7e, 0xa2, 0x0a, 0x9e, 0xee, 0xe6, 0x4b, 0x55, 0xd3, 0x9a,
		0x21, 0x92, 0x99, 0x2a, 0x27, 0x4f, 0xc1, 0xa8, 0x36, 0xba, 0x3c, 0x23, 0xa3, 0xfe, 0xeb, 0xbd,
		0x45, 0x4d, 0x44, 0x23, 0x64, 0x3c, 0xe8, 0x0e, 0x2a, 0x9a, 0xc9, 0x4f, 0xa5, 0x4c, 0xa4, 0x9f,
	];

	#[test]
	fn one_shot_matches_known_answer() {
		assert_eq!(sha256(b"abc"), ABC);
	}

	#[test]
	fn sha512_one_shot_matches_known_answer() {
		assert_eq!(sha512(b"abc"), ABC_512);
	}

	#[test]
	fn incremental_matches_one_shot() {
		let mut h = Sha256::new();
		h.update(b"a");
		h.update(b"bc");
		assert_eq!(h.finalize(), ABC);
	}
}
