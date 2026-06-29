use thiserror::Error;

/// Errors from AAuth token validation.
///
/// Errors from the underlying HTTP signing layer are forwarded transparently via `#[from]` on
/// [`http_message_sig::Error`]. Callers wanting to react to a specific signing error should match
/// on the embedded variant rather than parsing the display string.
#[derive(Debug, Error)]
pub enum AAuthError {
	#[error("HTTP signing error: {0}")]
	Signing(#[from] http_message_sig::Error),

	#[error("failed to fetch JWKS: {0}")]
	JwksFetch(String),

	#[error("JWT validation failed: {0}")]
	JwtValidation(String),

	#[error("audience mismatch")]
	AudienceMismatch,

	#[error("missing required claim: {0}")]
	MissingClaim(String),

	#[error("invalid issuer URL: must be HTTPS with host only (no port, path, query, or fragment)")]
	InvalidIssuerUrl,

	#[error("act.sub claim does not match agent identifier")]
	ActClaimMismatch,
}
