#![doc = include_str!("README.md")]

pub mod decoder;
pub(crate) mod parse;
pub(crate) mod render;
pub mod writer;

use crate::ir::events::ErrorKind;
use crate::protocols::{HttpReply, is_field_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorType {
	InvalidRequest,
	Authentication,
	Billing,
	Permission,
	NotFound,
	RequestTooLarge,
	RateLimit,
	Api,
	Overloaded,
}

impl ErrorType {
	#[must_use]
	pub const fn as_str(self) -> &'static str {
		match self {
			Self::InvalidRequest => "invalid_request_error",
			Self::Authentication => "authentication_error",
			Self::Billing => "billing_error",
			Self::Permission => "permission_error",
			Self::NotFound => "not_found_error",
			Self::RequestTooLarge => "request_too_large",
			Self::RateLimit => "rate_limit_error",
			Self::Api => "api_error",
			Self::Overloaded => "overloaded_error",
		}
	}

	const fn message(self) -> &'static str {
		match self {
			Self::InvalidRequest => "The provider refused the request.",
			Self::Authentication => "The gateway's provider credentials were refused.",
			Self::Billing => "The provider's quota is used up.",
			Self::Permission => "The provider refused access.",
			Self::NotFound => "The provider doesn't have this model.",
			Self::RequestTooLarge => "The request is too large for the provider.",
			Self::RateLimit => "The provider's rate limit was reached.",
			Self::Api => "The provider failed.",
			Self::Overloaded => "The provider is overloaded.",
		}
	}

	#[must_use]
	pub const fn status(self) -> u16 {
		match self {
			Self::InvalidRequest => 400,
			Self::Authentication => 401,
			Self::Billing => 402,
			Self::Permission => 403,
			Self::NotFound => 404,
			Self::RequestTooLarge => 413,
			Self::RateLimit => 429,
			Self::Api => 500,
			Self::Overloaded => 529,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
	pub kind: ErrorType,
	pub message: String,
}

impl Failure {
	pub fn new(kind: ErrorType, message: impl Into<String>) -> Self {
		Self {
			kind,
			message: message.into(),
		}
	}
}

/// The text is fixed: a provider's words can describe the gateway's account, not
/// the client's, so they stay in telemetry.
#[must_use]
pub fn classify(kind: ErrorKind, status: Option<u16>) -> Failure {
	let client = match kind {
		ErrorKind::ContextTooLong => {
			return Failure::new(
				ErrorType::InvalidRequest,
				"capability_rejected: prompt_too_long",
			);
		},
		ErrorKind::InvalidRequest => ErrorType::InvalidRequest,
		ErrorKind::RequestTooLarge => ErrorType::RequestTooLarge,
		ErrorKind::Authentication => ErrorType::Authentication,
		ErrorKind::Permission => ErrorType::Permission,
		ErrorKind::NotFound => ErrorType::NotFound,
		ErrorKind::Billing => ErrorType::Billing,
		ErrorKind::RateLimit => ErrorType::RateLimit,
		ErrorKind::Overloaded => ErrorType::Overloaded,
		ErrorKind::Unknown => match status {
			Some(400 | 422) => ErrorType::InvalidRequest,
			Some(401) => ErrorType::Authentication,
			Some(403) => ErrorType::Permission,
			Some(404) => ErrorType::NotFound,
			Some(413) => ErrorType::RequestTooLarge,
			Some(429) => ErrorType::RateLimit,
			Some(503 | 529) => ErrorType::Overloaded,
			_ => ErrorType::Api,
		},
	};
	Failure::new(client, client.message())
}

/// `classify`, naming the field a provider refused. A field name is the client's
/// own vocabulary, so it's safe to pass on where the provider's words aren't.
#[must_use]
pub fn classify_refused(kind: ErrorKind, status: Option<u16>, field: Option<&str>) -> Failure {
	let mut failure = classify(kind, status);
	if failure.kind == ErrorType::InvalidRequest
		&& failure.message == ErrorType::InvalidRequest.message()
		&& let Some(field) = field.filter(|field| is_field_name(field))
	{
		failure.message = format!("The provider refused `{field}`.");
	}
	failure
}

/// A provider's HTTP error as a client's. The status stays the provider's, so a
/// client's retry logic sees what the provider said.
#[must_use]
pub fn error_reply(status: u16, kind: ErrorKind) -> HttpReply {
	HttpReply {
		status,
		body: error_body(&classify(kind, Some(status))),
	}
}

#[must_use]
pub fn error_body(failure: &Failure) -> Vec<u8> {
	error_json(failure).into_bytes()
}

pub(super) fn error_json(failure: &Failure) -> String {
	format!(
		r#"{{"type":"error","error":{{"type":"{}","message":{}}}}}"#,
		failure.kind.as_str(),
		json_string(&failure.message)
	)
}

pub(super) fn json_string(text: &str) -> String {
	let mut out = String::with_capacity(text.len() + 2);
	crate::json::push_string(&mut out, text);
	out
}

#[cfg(test)]
mod tests {
	#![allow(clippy::unwrap_used)]

	use super::*;

	#[test]
	fn unknown_provider_codes_become_api_error() {
		assert_eq!(classify(ErrorKind::Unknown, None).kind, ErrorType::Api);
	}

	#[test]
	fn only_a_field_name_reaches_the_client() {
		let refused = |field| classify_refused(ErrorKind::Unknown, Some(400), Some(field)).message;
		assert_eq!(
			refused("temperature"),
			"The provider refused `temperature`."
		);
		assert_eq!(
			refused("input[3].content"),
			"The provider refused `input[3].content`."
		);
		for words in ["Project 1234 is over quota", "", "a`b", &"x".repeat(65)] {
			assert_eq!(refused(words), "The provider refused the request.");
		}
		let too_long = classify_refused(ErrorKind::ContextTooLong, Some(400), Some("input"));
		assert_eq!(too_long.message, "capability_rejected: prompt_too_long");
		let throttled = classify_refused(ErrorKind::RateLimit, Some(429), Some("input"));
		assert_eq!(throttled.message, "The provider's rate limit was reached.");
	}
}
