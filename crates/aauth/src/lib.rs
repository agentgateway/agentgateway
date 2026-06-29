//! AAuth protocol token validation (draft-hardt-oauth-aauth-protocol).
//!
//! This crate implements AAuth agent and auth token validation. The underlying HTTP signing layer
//! (RFC 9421 plus the HTTP Signature Keys draft) lives in [`http_message_sig`].

pub mod errors;
pub mod tokens;

pub use errors::AAuthError;
