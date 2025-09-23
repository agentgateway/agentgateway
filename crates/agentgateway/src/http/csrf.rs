use ::http::{Method, StatusCode, header};
use serde::de::Error;

use crate::http::{PolicyResponse, Request, filters};
use crate::*;

/// String matcher for CSRF additional origins
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StringMatcher {
	Exact(String),
	Prefix(String),
	Suffix(String),
	Contains(String),
	SafeRegex(String),
}

impl StringMatcher {
	/// Check if the given value matches this matcher
	pub fn matches(&self, value: &str) -> bool {
		match self {
			StringMatcher::Exact(exact) => value == exact,
			StringMatcher::Prefix(prefix) => value.starts_with(prefix),
			StringMatcher::Suffix(suffix) => value.ends_with(suffix),
			StringMatcher::Contains(contains) => value.contains(contains),
			StringMatcher::SafeRegex(regex) => regex::Regex::new(regex)
				.map(|r| r.is_match(value))
				.unwrap_or(false),
		}
	}
}

#[derive(Default, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StringMatcherSerde {
	pub exact: Option<String>,
	pub prefix: Option<String>,
	pub suffix: Option<String>,
	pub contains: Option<String>,
	pub safe_regex: Option<String>,
	#[serde(default)]
	pub ignore_case: bool,
}

impl TryFrom<StringMatcherSerde> for StringMatcher {
	type Error = anyhow::Error;

	fn try_from(value: StringMatcherSerde) -> Result<Self, Self::Error> {
		let mut count = 0;
		let mut result = None;

		if let Some(exact) = value.exact {
			count += 1;
			result = Some(StringMatcher::Exact(exact));
		}
		if let Some(prefix) = value.prefix {
			count += 1;
			result = Some(StringMatcher::Prefix(prefix));
		}
		if let Some(suffix) = value.suffix {
			count += 1;
			result = Some(StringMatcher::Suffix(suffix));
		}
		if let Some(contains) = value.contains {
			count += 1;
			result = Some(StringMatcher::Contains(contains));
		}
		if let Some(safe_regex) = value.safe_regex {
			count += 1;
			result = Some(StringMatcher::SafeRegex(safe_regex));
		}

		if count != 1 {
			anyhow::bail!("Exactly one of exact, prefix, suffix, contains, or safe_regex must be set");
		}

		result.ok_or_else(|| anyhow::anyhow!("No matcher type set"))
	}
}

#[apply(schema_ser!)]
#[cfg_attr(feature = "schema", schemars(with = "CsrfSerde"))]
pub struct Csrf {
	/// Additional origins that are allowed for CSRF validation
	#[serde(default)]
	additional_origins: Vec<StringMatcher>,
}

impl<'de> serde::Deserialize<'de> for Csrf {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Csrf::try_from(CsrfSerde::deserialize(deserializer)?).map_err(D::Error::custom)
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CsrfSerde {
	/// Additional origins that are allowed for CSRF validation
	#[serde(default)]
	pub additional_origins: Vec<StringMatcherSerde>,
}

impl TryFrom<CsrfSerde> for Csrf {
	type Error = anyhow::Error;

	fn try_from(value: CsrfSerde) -> Result<Self, Self::Error> {
		let additional_origins = value
			.additional_origins
			.into_iter()
			.map(StringMatcher::try_from)
			.collect::<Result<Vec<_>, _>>()?;

		Ok(Csrf { additional_origins })
	}
}

impl Csrf {
	/// Apply CSRF validation to the request
	/// Returns a PolicyResponse indicating whether the request should be allowed
	pub fn apply(&self, req: &mut Request) -> Result<PolicyResponse, filters::Error> {
		// Only apply CSRF protection to modifying methods
		if !is_modify_method(req.method()) {
			return Ok(Default::default());
		}

		let source_origin = source_origin_value(req)?;
		if source_origin.is_empty() {
			// No source origin found, this is suspicious for modifying requests
			// Return 403 Forbidden as per Envoy behavior
			let response = ::http::Response::builder()
				.status(StatusCode::FORBIDDEN)
				.body(crate::http::Body::from("Invalid origin"))?;
			return Ok(PolicyResponse {
				direct_response: Some(response),
				response_headers: None,
			});
		}

		if !self.is_valid(&source_origin, req)? {
			// Origin validation failed
			let response = ::http::Response::builder()
				.status(StatusCode::FORBIDDEN)
				.body(crate::http::Body::from("Invalid origin"))?;
			return Ok(PolicyResponse {
				direct_response: Some(response),
				response_headers: None,
			});
		}

		// Origin validation passed
		Ok(Default::default())
	}

	/// Check if the source origin is valid for this request
	fn is_valid(&self, source_origin: &str, req: &Request) -> Result<bool, filters::Error> {
		let target_origin = target_origin_value(req)?;

		// First check if source and target origins match exactly
		if source_origin == target_origin {
			return Ok(true);
		}

		// Check against additional origins
		for matcher in self.additional_origins.iter() {
			if matcher.matches(source_origin) {
				return Ok(true);
			}
		}

		Ok(false)
	}
}

/// Check if the HTTP method is a modifying method that requires CSRF protection
fn is_modify_method(method: &Method) -> bool {
	matches!(
		method,
		&Method::POST | &Method::PUT | &Method::DELETE | &Method::PATCH
	)
}

/// Extract the source origin value from request headers
fn source_origin_value(req: &Request) -> Result<String, filters::Error> {
	// Try Origin header first
	if let Some(origin_value) = req.headers().get(header::ORIGIN) {
		let origin_str = origin_value.to_str().map_err(|_| {
			filters::Error::InvalidFilterConfiguration("Invalid Origin header".to_string())
		})?;

		// Handle "null" origin
		if origin_str == "null" {
			return Ok(String::new());
		}

		return Ok(origin_str.to_string());
	}

	// Fall back to Referer header
	if let Some(referer_value) = req.headers().get(header::REFERER) {
		let referer_str = referer_value.to_str().map_err(|_| {
			filters::Error::InvalidFilterConfiguration("Invalid Referer header".to_string())
		})?;
		// For referer, extract the origin part (scheme + host + port)
		let origin = extract_origin_from_url(referer_str);
		return Ok(origin);
	}

	Ok(String::new())
}

/// Extract the target origin value from the request
fn target_origin_value(req: &Request) -> Result<String, filters::Error> {
	// After URI normalization, the Host header is removed and the host info
	// is moved to req.uri().authority().
	if let Some(authority) = req.uri().authority() {
		let scheme = req.uri().scheme_str().unwrap_or("http");
		let full_origin = format!("{}://{}", scheme, authority);
		return Ok(full_origin);
	}

	let host_value = req.headers().get(header::HOST).ok_or_else(|| {
		filters::Error::InvalidFilterConfiguration("Missing target origin information".to_string())
	})?;

	let host_str = host_value
		.to_str()
		.map_err(|_| filters::Error::InvalidFilterConfiguration("Invalid Host header".to_string()))?;

	if host_str.is_empty() {
		return Ok(String::new());
	}

	// Construct the full origin from the request scheme and host
	// Default to http if no scheme is present in the request URI
	let scheme = if req.uri().scheme().is_some() {
		req.uri().scheme_str().unwrap_or("http")
	} else {
		"http"
	};

	let full_origin = format!("{}://{}", scheme, host_str);
	Ok(full_origin)
}

/// Extract origin (scheme + host + port) from a full URL
fn extract_origin_from_url(url: &str) -> String {
	if url.is_empty() {
		return String::new();
	}

	// Parse the URL and extract origin
	if let Ok(uri) = url.parse::<::http::Uri>()
		&& let Some(authority) = uri.authority()
		&& let Some(scheme) = uri.scheme_str()
	{
		let origin = format!("{}://{}", scheme, authority);
		return origin;
	}

	// If parsing fails, return empty
	String::new()
}
