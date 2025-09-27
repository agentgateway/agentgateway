use ::http::{Method, StatusCode, header};
use std::collections::HashSet;

use crate::http::{PolicyResponse, Request, filters};
use crate::*;

#[apply(schema)]
pub struct Csrf {
	#[serde(default)]
	additional_origins: HashSet<String>,
}

impl Csrf {
	pub fn apply(&self, req: &mut Request) -> Result<PolicyResponse, filters::Error> {
		// 1. Allow all GET, HEAD, or OPTIONS requests
		if is_safe_method(req.method()) {
			return Ok(Default::default());
		}

		// 2. Check Sec-Fetch-Site header first
		match get_sec_fetch_site_header(req)? {
			Some(sec_fetch_site) => {
				match sec_fetch_site.as_str() {
					"same-origin" | "none" => return Ok(Default::default()),
					_ => {
						// Check if request is exempt (trusted origins)
						if self.is_request_exempt(req)? {
							return Ok(Default::default());
						}
						return self.create_forbidden_response(
							"Cross-origin request detected from Sec-Fetch-Site header".to_string(),
						);
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
			return Ok(Default::default());
		};

		// 5. Check if Origin matches Host header
		let target_origin = get_target_origin(req)?;
		if origin == target_origin {
			return Ok(Default::default());
		}

		// 6. Check trusted origins as last resort
		if self.is_request_exempt(req)? {
			return Ok(Default::default());
		}

		// Request failed all checks - reject
		self.create_forbidden_response("Cross-origin request detected".to_string())
	}

	fn is_request_exempt(&self, req: &Request) -> Result<bool, filters::Error> {
		if let Some(origin) = get_origin_header(req)? {
			return Ok(self.additional_origins.contains(&origin));
		}
		Ok(false)
	}

	/// Create a 403 Forbidden response
	fn create_forbidden_response(&self, message: String) -> Result<PolicyResponse, filters::Error> {
		let response = ::http::Response::builder()
			.status(StatusCode::FORBIDDEN)
			.body(crate::http::Body::from(message))?;
		Ok(PolicyResponse {
			direct_response: Some(response),
			response_headers: None,
		})
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
			filters::Error::InvalidFilterConfiguration("Invalid Origin header".to_string())
		})?;

		// Handle "null" origin as no origin
		if origin_str == "null" {
			return Ok(None);
		}

		return Ok(Some(origin_str.to_string()));
	}
	Ok(None)
}

/// Extract the Sec-Fetch-Site header value
fn get_sec_fetch_site_header(req: &Request) -> Result<Option<String>, filters::Error> {
	if let Some(sec_fetch_site_value) = req.headers().get("sec-fetch-site") {
		let sec_fetch_site_str = sec_fetch_site_value.to_str().map_err(|_| {
			filters::Error::InvalidFilterConfiguration("Invalid Sec-Fetch-Site header".to_string())
		})?;
		return Ok(Some(sec_fetch_site_str.to_string()));
	}
	Ok(None)
}

/// Extract the target origin from the request
fn get_target_origin(req: &Request) -> Result<String, filters::Error> {
	let authority = req.uri().authority().ok_or_else(|| {
		filters::Error::InvalidFilterConfiguration("Missing authority in URI".to_string())
	})?;
	let scheme = req.uri().scheme_str().unwrap_or("http");
	Ok(format!("{}://{}", scheme, authority))
}
