use thiserror::Error;

/// Errors from RFC 9421 HTTP Message Signatures and the HTTP Signature Keys spec.
#[derive(Debug, Error)]
pub enum Error {
	#[error("missing Signature-Key header")]
	MissingSignatureKey,

	#[error("missing Signature-Input header")]
	MissingSignatureInput,

	#[error("missing Signature header")]
	MissingSignature,

	#[error("label mismatch across headers")]
	LabelMismatch,

	#[error("signature-key must be a covered component")]
	SignatureKeyNotCovered,

	#[error("signature created timestamp outside valid window")]
	TimestampExpired,

	#[error("signature verification failed: {0}")]
	InvalidSignature(String),

	#[error("unsupported signature scheme: {0}")]
	UnsupportedScheme(String),

	#[error("unsupported algorithm: {0}")]
	UnsupportedAlgorithm(String),

	#[error("content-digest verification failed")]
	ContentDigestMismatch,

	#[error("invalid header format: {0}")]
	InvalidHeader(String),

	#[error("invalid key format: {0}")]
	InvalidKey(String),

	#[error("base64 decode error: {0}")]
	Base64Error(#[from] base64::DecodeError),

	#[error("URL parse error: {0}")]
	UrlError(#[from] url::ParseError),

	#[error("JSON parse error: {0}")]
	JsonError(#[from] serde_json::Error),
}
