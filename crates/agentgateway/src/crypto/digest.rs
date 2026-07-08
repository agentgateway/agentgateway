//! Cryptographic hash functions.
//!
//! This is the seam for hashing within the `agentgateway` crate. It is
//! currently backed by `aws-lc-rs`, which is always linked regardless of the
//! selected TLS provider feature. Additional backends (e.g. SymCrypt) plug in
//! here behind `#[cfg]` without changing call sites.
//!
//! Note: hashing in sibling crates (`agent-celx`, `htpasswd-verify-fork`) still
//! uses the RustCrypto crates directly, as they cannot depend on this crate.
//! Consolidating those requires a shared crypto crate and is tracked separately.

use aws_lc_rs::digest::{self, Context, SHA256};

/// Length in bytes of a SHA-256 digest.
pub const SHA256_LEN: usize = 32;

/// Computes the SHA-256 digest of `data` in one shot.
pub fn sha256(data: &[u8]) -> [u8; SHA256_LEN] {
	to_array(digest::digest(&SHA256, data).as_ref())
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

fn to_array(bytes: &[u8]) -> [u8; SHA256_LEN] {
	let mut out = [0u8; SHA256_LEN];
	out.copy_from_slice(bytes);
	out
}

#[cfg(test)]
mod tests {
	use super::{Sha256, sha256};

	// SHA-256("abc") known-answer vector (FIPS 180-4).
	const ABC: [u8; 32] = [
		0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
		0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
	];

	#[test]
	fn one_shot_matches_known_answer() {
		assert_eq!(sha256(b"abc"), ABC);
	}

	#[test]
	fn incremental_matches_one_shot() {
		let mut h = Sha256::new();
		h.update(b"a");
		h.update(b"bc");
		assert_eq!(h.finalize(), ABC);
	}
}
