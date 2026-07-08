//! Cryptographically-secure random number generation.
//!
//! This is the single seam for CSPRNG output in agentgateway. It is currently
//! backed by `aws-lc-rs`, which is always linked regardless of the selected TLS
//! provider feature. Additional backends (e.g. SymCrypt) plug in here behind
//! `#[cfg]` without changing call sites.

use aws_lc_rs::rand::{SecureRandom, SystemRandom};

/// Fills `dest` with cryptographically-secure random bytes.
///
/// Returns [`RandError`] only if the system CSPRNG fails, which callers should
/// treat as unrecoverable.
pub fn fill(dest: &mut [u8]) -> Result<(), RandError> {
	SystemRandom::new().fill(dest).map_err(|_| RandError)
}

/// Returns `len` cryptographically-secure random bytes.
pub fn bytes(len: usize) -> Result<Vec<u8>, RandError> {
	let mut out = vec![0u8; len];
	fill(&mut out)?;
	Ok(out)
}

#[derive(Debug, thiserror::Error)]
#[error("secure random generation failed")]
pub struct RandError;

#[cfg(test)]
mod tests {
	use super::{bytes, fill};

	#[test]
	fn fill_produces_nonzero_and_distinct() {
		let mut a = [0u8; 32];
		let mut b = [0u8; 32];
		fill(&mut a).expect("fill a");
		fill(&mut b).expect("fill b");
		assert_ne!(a, [0u8; 32], "output should not be all zeros");
		assert_ne!(a, b, "two draws should differ");
	}

	#[test]
	fn bytes_returns_requested_len() {
		assert_eq!(bytes(16).expect("bytes").len(), 16);
	}
}
