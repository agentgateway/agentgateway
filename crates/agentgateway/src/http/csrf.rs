use std::collections::HashSet;

use ::http::{Method, header};

use crate::http::{PolicyResponse, Request, filters};
use crate::proxy::ProxyError;
use crate::*;

#[apply(schema)]
pub struct Csrf {
	/// Additional trusted origins allowed to send state-changing requests.
	#[serde(default)]
	additional_origins: HashSet<String>,
}

impl Csrf {
	/// Create a new CSRF policy with the given additional trusted origins
	pub fn new(additional_origins: HashSet<String>) -> Self {
		Self { additional_origins }
	}

	/// Returns whether the request passes CSRF validation.
	pub fn is_allowed(&self, req: &Request) -> Result<bool, filters::Error> {
		// 1. Allow all GET, HEAD, or OPTIONS requests
		if is_safe_method(req.method()) {
			return Ok(true);
		}

		// 2. Check Sec-Fetch-Site header first
		match get_sec_fetch_site_header(req)? {
			Some(sec_fetch_site) => {
				match sec_fetch_site.as_str() {
					"same-origin" | "none" => return Ok(true),
					_ => {
						// Check if request is exempt (trusted origins)
						if self.is_request_exempt(req)? {
							return Ok(true);
						}
						// Log detailed reason internally for debugging
						warn!(
							"CSRF validation failed: Sec-Fetch-Site header indicates cross-site request: {}",
							sec_fetch_site
						);
						return Ok(false);
					},
				}
			},
			None => {
				// No Sec-Fetch-Site header - fallthrough to Origin check
			},
		}

		// 3. Get Origin header once
		let Some(origin) = get_origin_header(req)? else {
			// If no Origin header, allow
			return Ok(true);
		};

		// 5. Check if Origin host matches Host header. The scheme is ignored, since a TLS-terminating
		// proxy in front of us means we cannot know it. Old browsers lacking Sec-Fetch-Site can rely on HSTS.
		let target_host = req.uri().authority().ok_or_else(|| {
			filters::Error::InvalidFilterConfiguration("missing authority in URI".to_string())
		})?;
		let origin_uri = origin.parse::<::http::Uri>().ok();
		if origin_uri
			.as_ref()
			.filter(|u| u.scheme().is_some())
			.and_then(|u| u.authority())
			== Some(target_host)
		{
			return Ok(true);
		}

		// 6. Check trusted origins as last resort
		if self.is_request_exempt(req)? {
			return Ok(true);
		}

		// Log detailed reason internally for debugging
		warn!(
			"CSRF validation failed: Origin '{}' does not match target host '{}'",
			origin, target_host
		);
		// Request failed all checks - reject
		Ok(false)
	}

	fn is_request_exempt(&self, req: &Request) -> Result<bool, filters::Error> {
		if let Some(origin) = get_origin_header(req)? {
			return Ok(self.additional_origins.contains(&origin));
		}
		Ok(false)
	}
}

impl crate::store::RequestPolicyTrait for Csrf {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		match self.is_allowed(req) {
			Ok(true) => Ok(Default::default()),
			_ => Err(ProxyError::CsrfValidationFailed),
		}
	}
}

/// Check if the HTTP method is a safe method
fn is_safe_method(method: &Method) -> bool {
	matches!(method, &Method::GET | &Method::HEAD | &Method::OPTIONS)
}

/// Extract the Origin header value
fn get_origin_header(req: &Request) -> Result<Option<String>, filters::Error> {
	if let Some(origin_value) = req.headers().get(header::ORIGIN) {
		let origin_str = origin_value.to_str().map_err(|_| {
			filters::Error::InvalidFilterConfiguration("malformed origin header".to_string())
		})?;
		return Ok(Some(origin_str.to_string()));
	}
	Ok(None)
}

/// Extract the Sec-Fetch-Site header value
fn get_sec_fetch_site_header(req: &Request) -> Result<Option<String>, filters::Error> {
	if let Some(sec_fetch_site_value) = req.headers().get("sec-fetch-site") {
		let sec_fetch_site_str = sec_fetch_site_value.to_str().map_err(|_| {
			filters::Error::InvalidFilterConfiguration("malformed Sec-Fetch-Site header".to_string())
		})?;
		return Ok(Some(sec_fetch_site_str.to_string()));
	}
	Ok(None)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn is_allowed() {
		let csrf = Csrf::new(HashSet::from(["https://trusted.example".to_string()]));
		let cases = [
			("GET", None, Some("https://evil.example"), true),
			(
				"POST",
				Some("same-origin"),
				Some("https://evil.example"),
				true,
			),
			("POST", Some("none"), None, true),
			(
				"POST",
				Some("cross-site"),
				Some("https://evil.example"),
				false,
			),
			(
				"POST",
				Some("cross-site"),
				Some("https://trusted.example"),
				true,
			),
			("POST", None, Some("https://app.example"), true),
			("POST", None, Some("https://evil.example"), false),
			("POST", None, None, true),
		];
		for (method, fetch_site, origin, want) in cases {
			let mut req = ::http::Request::builder()
				.method(method)
				.uri("https://app.example/");
			if let Some(fetch_site) = fetch_site {
				req = req.header("sec-fetch-site", fetch_site);
			}
			if let Some(origin) = origin {
				req = req.header(header::ORIGIN, origin);
			}
			let req = req.body(crate::http::Body::empty()).unwrap();
			assert_eq!(
				csrf.is_allowed(&req).unwrap(),
				want,
				"{method} sec-fetch-site={fetch_site:?} origin={origin:?}"
			);
		}
	}
}
