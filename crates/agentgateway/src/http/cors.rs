use std::str::FromStr;

use ::http::{HeaderValue, Method, StatusCode, header};
use serde::de::Error;

use crate::http::{PolicyResponse, Request, filters};
use crate::*;

#[derive(Default, Debug, Clone)]
enum WildcardOrList<T> {
	#[default]
	None,
	Wildcard,
	List(Vec<T>),
}

impl<T> WildcardOrList<T> {
	fn is_none(&self) -> bool {
		matches!(self, WildcardOrList::None)
	}
}

impl<T: FromStr> TryFrom<Vec<String>> for WildcardOrList<T> {
	type Error = T::Err;

	fn try_from(value: Vec<String>) -> Result<Self, Self::Error> {
		if value.contains(&"*".to_string()) {
			Ok(WildcardOrList::Wildcard)
		} else if value.is_empty() {
			Ok(WildcardOrList::None)
		} else {
			let vec: Vec<T> = value
				.into_iter()
				.map(|v| T::from_str(&v))
				.collect::<Result<_, _>>()?;
			Ok(WildcardOrList::List(vec))
		}
	}
}

impl<T: Display> Serialize for WildcardOrList<T> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			WildcardOrList::None => Vec::<String>::new().serialize(serializer),
			WildcardOrList::Wildcard => vec!["*"].serialize(serializer),
			WildcardOrList::List(list) => list
				.iter()
				.map(ToString::to_string)
				.collect::<Vec<_>>()
				.serialize(serializer),
		}
	}
}

impl<T> WildcardOrList<T>
where
	T: ToString,
{
	fn to_header_value(&self) -> Option<::http::HeaderValue> {
		match self {
			WildcardOrList::None => None,
			WildcardOrList::Wildcard => Some(::http::HeaderValue::from_static("*")),
			WildcardOrList::List(list) => {
				let value = list
					.iter()
					.map(|item| item.to_string())
					.collect::<Vec<_>>()
					.join(",");

				::http::HeaderValue::from_str(&value).ok()
			},
		}
	}
}

#[apply(schema_ser!)]
#[cfg_attr(feature = "schema", schemars(with = "CorsSerde"))]
pub struct Cors {
	allow_credentials: bool,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	allow_headers: WildcardOrList<http::HeaderName>,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	allow_methods: WildcardOrList<::http::Method>,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	allow_origins: WildcardOrList<Strng>,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	expose_headers: WildcardOrList<http::HeaderName>,
	#[serde(serialize_with = "ser_string_or_bytes_option")]
	max_age: Option<::http::HeaderValue>,
}

impl<'de> serde::Deserialize<'de> for Cors {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Cors::try_from(CorsSerde::deserialize(deserializer)?).map_err(D::Error::custom)
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CorsSerde {
	#[serde(default)]
	pub allow_credentials: bool,
	#[serde(default)]
	pub allow_headers: Vec<String>,
	#[serde(default)]
	pub allow_methods: Vec<String>,
	#[serde(default)]
	pub allow_origins: Vec<String>,
	#[serde(default)]
	pub expose_headers: Vec<String>,
	#[serde(default, with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub max_age: Option<Duration>,
}

impl TryFrom<CorsSerde> for Cors {
	type Error = anyhow::Error;
	fn try_from(value: CorsSerde) -> Result<Self, Self::Error> {
		Ok(Cors {
			allow_credentials: value.allow_credentials,
			allow_headers: WildcardOrList::try_from(value.allow_headers)?,
			allow_methods: WildcardOrList::try_from(value.allow_methods)?,
			allow_origins: WildcardOrList::try_from(value.allow_origins)?,
			expose_headers: WildcardOrList::try_from(value.expose_headers)?,
			max_age: value
				.max_age
				.map(|v| http::HeaderValue::from_str(&v.as_secs().to_string()))
				.transpose()?,
		})
	}
}

impl Cors {
	/// Apply applies the CORS header. It seems a lot of implementations handle this differently wrt when
	/// to add or not add headers, and when to forward the request.
	/// We follow Envoy semantics here (with forwardNotMatchingPreflights=false)
	pub fn apply(&self, req: &mut Request) -> Result<PolicyResponse, filters::Error> {
		// If no origin, return immediately
		let Some(origin) = req.headers().get(header::ORIGIN) else {
			return Ok(Default::default());
		};
		// Determine whether this is a CORS preflight request:
		// - method is OPTIONS
		// - Origin is present (already true here)
		// - Access-Control-Request-Method is present and non-empty
		let is_preflight = req.method() == Method::OPTIONS
			&& req
				.headers()
				.get(header::ACCESS_CONTROL_REQUEST_METHOD)
				.map(|v| !v.as_bytes().is_empty())
				.unwrap_or(false);
		let parsed_origin = origin
			.to_str()
			.ok()
			.and_then(|value| ParsedOrigin::parse(value, false));

		let origin_allowed = match &self.allow_origins {
			WildcardOrList::None => false,
			WildcardOrList::Wildcard => true,
			WildcardOrList::List(origins) => parsed_origin.as_ref().is_some_and(|request_origin| {
				origins
					.iter()
					.any(|allowed_origin| matches_allowed_origin(allowed_origin.as_str(), request_origin))
			}),
		};

		if !origin_allowed {
			if is_preflight {
				// Semantics: do not forward non-matching preflight requests.
				// If it is a preflight and the origin does not match, respond locally with 200 and no CORS headers.
				let response = ::http::Response::builder()
					.status(StatusCode::OK)
					.body(crate::http::Body::empty())?;
				return Ok(PolicyResponse {
					direct_response: Some(response),
					response_headers: None,
				});
			} else {
				// If not a preflight, and origin is not allowed, do nothing (let it pass through without CORS headers).
				return Ok(Default::default());
			}
		}
		// If not a preflight, and origin is not allowed, do nothing (let it pass through without CORS headers).
		if !is_preflight && !origin_allowed {
			return Ok(Default::default());
		}

		if req.method() == Method::OPTIONS {
			// Handle preflight request
			let mut rb = ::http::Response::builder()
				.status(StatusCode::OK)
				.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
			if let Some(h) = self.preflight_allow_methods(req.headers()) {
				rb = rb.header(header::ACCESS_CONTROL_ALLOW_METHODS, h);
			}
			if let Some(h) = self.preflight_allow_headers(req.headers()) {
				rb = rb.header(header::ACCESS_CONTROL_ALLOW_HEADERS, h);
			}
			if let Some(h) = &self.max_age {
				rb = rb.header(header::ACCESS_CONTROL_MAX_AGE, h);
			}
			if self.allow_credentials {
				rb = rb.header(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HEADER_VALUE_TRUE);
			}
			if let Some(h) = self.expose_headers.to_header_value() {
				rb = rb.header(header::ACCESS_CONTROL_EXPOSE_HEADERS, h);
			}
			let response = rb.body(crate::http::Body::empty())?;
			return Ok(PolicyResponse {
				direct_response: Some(response),
				response_headers: None,
			});
		}

		let mut response_headers = http::HeaderMap::with_capacity(3);
		response_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
		if self.allow_credentials {
			response_headers.insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HEADER_VALUE_TRUE);
		}
		if let Some(h) = self.expose_headers.to_header_value() {
			response_headers.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, h);
		}
		// For actual requests, we would need to add CORS headers to the response
		// but since we only have access to the request here, we return None
		Ok(PolicyResponse {
			direct_response: None,
			response_headers: Some(response_headers),
		})
	}

	fn preflight_allow_methods(&self, headers: &http::HeaderMap) -> Option<http::HeaderValue> {
		match &self.allow_methods {
			WildcardOrList::None => None,
			WildcardOrList::Wildcard => headers
				.get(header::ACCESS_CONTROL_REQUEST_METHOD)
				.and_then(normalize_token_header_value)
				.or_else(|| {
					if self.allow_credentials {
						None
					} else {
						Some(http::HeaderValue::from_static("*"))
					}
				}),
			WildcardOrList::List(_) => self.allow_methods.to_header_value(),
		}
	}

	fn preflight_allow_headers(&self, headers: &http::HeaderMap) -> Option<http::HeaderValue> {
		match &self.allow_headers {
			WildcardOrList::None => None,
			WildcardOrList::Wildcard => headers
				.get(header::ACCESS_CONTROL_REQUEST_HEADERS)
				.and_then(normalize_csv_header_value)
				.or_else(|| {
					if self.allow_credentials {
						None
					} else {
						Some(http::HeaderValue::from_static("*"))
					}
				}),
			WildcardOrList::List(_) => self.allow_headers.to_header_value(),
		}
	}
}

const HEADER_VALUE_TRUE: http::HeaderValue = HeaderValue::from_static("true");

fn normalize_token_header_value(value: &http::HeaderValue) -> Option<http::HeaderValue> {
	let value = value.to_str().ok()?.trim();
	if value.is_empty() {
		return None;
	}
	http::HeaderValue::from_str(value).ok()
}

fn normalize_csv_header_value(value: &http::HeaderValue) -> Option<http::HeaderValue> {
	let value = value.to_str().ok()?;
	let normalized = value
		.split(',')
		.map(str::trim)
		.filter(|entry| !entry.is_empty())
		.collect::<Vec<_>>()
		.join(", ");
	if normalized.is_empty() {
		return None;
	}
	http::HeaderValue::from_str(&normalized).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OriginScheme {
	Http,
	Https,
}

impl OriginScheme {
	fn parse(value: &str) -> Option<Self> {
		if value.eq_ignore_ascii_case("http") {
			Some(Self::Http)
		} else if value.eq_ignore_ascii_case("https") {
			Some(Self::Https)
		} else {
			None
		}
	}

	fn default_port(self) -> u16 {
		match self {
			Self::Http => 80,
			Self::Https => 443,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedOrigin {
	scheme: OriginScheme,
	host: String,
	port: u16,
}

impl ParsedOrigin {
	fn parse(value: &str, allow_wildcard_host: bool) -> Option<Self> {
		let (scheme, host_port) = value.split_once("://")?;
		let scheme = OriginScheme::parse(scheme)?;
		if host_port.is_empty() || host_port.chars().any(|c| matches!(c, '/' | '?' | '#')) {
			return None;
		}

		let (host, port) = match host_port.rsplit_once(':') {
			Some((host, port)) => {
				if host.is_empty()
					|| host.contains(':')
					|| port.is_empty()
					|| !port.bytes().all(|b| b.is_ascii_digit())
				{
					return None;
				}
				let port = port.parse::<u16>().ok()?;
				if port == 0 {
					return None;
				}
				(host, port)
			},
			None => (host_port, scheme.default_port()),
		};

		if host.is_empty() || (!allow_wildcard_host && host.contains('*')) {
			return None;
		}

		Some(Self {
			scheme,
			host: host.to_ascii_lowercase(),
			port,
		})
	}
}

fn matches_allowed_origin(allowed_origin: &str, request_origin: &ParsedOrigin) -> bool {
	let Some(allowed_origin) = ParsedOrigin::parse(allowed_origin, true) else {
		return false;
	};

	allowed_origin.scheme == request_origin.scheme
		&& allowed_origin.port == request_origin.port
		&& host_matches(&allowed_origin.host, &request_origin.host)
}

fn host_matches(pattern: &str, host: &str) -> bool {
	if pattern == "*" {
		return true;
	}
	if !pattern.contains('*') {
		return pattern == host;
	}
	wildcard_match(pattern, host)
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
	let pattern = pattern.as_bytes();
	let value = value.as_bytes();
	let (mut pattern_idx, mut value_idx) = (0usize, 0usize);
	let mut star_idx = None;
	let mut star_match_idx = 0usize;

	while value_idx < value.len() {
		if pattern_idx < pattern.len()
			&& (pattern[pattern_idx] == value[value_idx] || pattern[pattern_idx] == b'*')
		{
			if pattern[pattern_idx] == b'*' {
				star_idx = Some(pattern_idx);
				star_match_idx = value_idx;
				pattern_idx += 1;
			} else {
				pattern_idx += 1;
				value_idx += 1;
			}
		} else if let Some(star_pos) = star_idx {
			pattern_idx = star_pos + 1;
			star_match_idx += 1;
			value_idx = star_match_idx;
		} else {
			return false;
		}
	}

	while pattern_idx < pattern.len() && pattern[pattern_idx] == b'*' {
		pattern_idx += 1;
	}

	pattern_idx == pattern.len()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_origin_applies_default_port() {
		let parsed = ParsedOrigin::parse("http://example.com", false).expect("valid origin");
		assert_eq!(parsed.port, 80);
		let parsed = ParsedOrigin::parse("https://example.com", false).expect("valid origin");
		assert_eq!(parsed.port, 443);
	}

	#[test]
	fn default_and_explicit_ports_match() {
		let req = ParsedOrigin::parse("http://example.com", false).expect("valid origin");
		assert!(matches_allowed_origin("http://example.com:80", &req));

		let req = ParsedOrigin::parse("https://example.com:443", false).expect("valid origin");
		assert!(matches_allowed_origin("https://example.com", &req));
	}

	#[test]
	fn wildcard_host_matches_com_suffix() {
		let req = ParsedOrigin::parse("http://foo.bar.com", false).expect("valid origin");
		assert!(matches_allowed_origin("http://*.com", &req));
		assert!(matches_allowed_origin("http://*.bar.com", &req));
		assert!(!matches_allowed_origin("http://*.org", &req));
		assert!(!matches_allowed_origin("https://*.com", &req));
	}

	#[test]
	fn wildcard_host_can_match_all_hosts_for_scheme() {
		let req = ParsedOrigin::parse("https://service.internal", false).expect("valid origin");
		assert!(matches_allowed_origin("https://*", &req));
		assert!(!matches_allowed_origin("http://*", &req));
	}

	#[test]
	fn wildcard_host_respects_explicit_port() {
		let req = ParsedOrigin::parse("https://foo.com:8443", false).expect("valid origin");
		assert!(matches_allowed_origin("https://*.com:8443", &req));
		assert!(!matches_allowed_origin("https://*.com", &req));
	}

	#[test]
	fn parse_origin_rejects_invalid_values() {
		assert!(ParsedOrigin::parse("ftp://example.com", false).is_none());
		assert!(ParsedOrigin::parse("http://example.com/path", false).is_none());
		assert!(ParsedOrigin::parse("http://example.com:0", false).is_none());
		assert!(ParsedOrigin::parse("http://exa*mple.com", false).is_none());
	}

	#[test]
	fn preflight_wildcard_headers_echo_request_headers() {
		let cors = Cors::try_from(CorsSerde {
			allow_credentials: false,
			allow_headers: vec!["*".to_string()],
			allow_methods: vec!["*".to_string()],
			allow_origins: vec!["*".to_string()],
			expose_headers: vec![],
			max_age: None,
		})
		.expect("valid cors policy");
		let mut req = ::http::Request::builder()
			.method(Method::OPTIONS)
			.uri("http://lo")
			.header(header::ORIGIN, "http://example.com")
			.header(header::ACCESS_CONTROL_REQUEST_METHOD, "PUT")
			.header(
				header::ACCESS_CONTROL_REQUEST_HEADERS,
				"x-header-1, x-header-2",
			)
			.body(crate::http::Body::empty())
			.expect("valid request");

		let response = cors.apply(&mut req).expect("cors evaluation");
		let direct = response.direct_response.expect("preflight response");
		assert_eq!(
			direct
				.headers()
				.get(header::ACCESS_CONTROL_ALLOW_METHODS)
				.and_then(|v| v.to_str().ok()),
			Some("PUT")
		);
		assert_eq!(
			direct
				.headers()
				.get(header::ACCESS_CONTROL_ALLOW_HEADERS)
				.and_then(|v| v.to_str().ok()),
			Some("x-header-1, x-header-2")
		);
	}
}
