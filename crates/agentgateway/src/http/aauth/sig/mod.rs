//! HTTP Message Signatures (RFC 9421) and HTTP Signature Keys spec implementation.
//!
//! This module provides:
//! - RFC 9421: HTTP Message Signatures — signing and verification
//! - RFC 9530: Content-Digest header
//! - draft-hardt-httpbis-signature-key: Signature-Key header (hwk, jwks_uri, jwt schemes)
//! - Ed25519 key support (RFC 8037)
//! - Jwk key format (RFC 7517) and thumbprints (RFC 7638)

pub mod digest;
pub mod encoding;
pub mod errors;
pub mod headers;
pub mod keys;
pub mod signing;

#[cfg(test)]
mod test_vectors;

pub use errors::Error;
