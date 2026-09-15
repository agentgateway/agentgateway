//! Configurable browser login and logout endpoints.
use super::{Error, OidcPolicy, build_redirect_response, session};
use crate::http::{Body, PolicyResponse, Request};
use ::http::{Method, StatusCode, Uri, header};

impl OidcPolicy {
	pub(super) fn return_target(&self, uri: &Uri) -> String {
		let target = crate::http::query_parameter(uri, "returnTo")
			.and_then(|value| value.parse::<http::uri::PathAndQuery>().ok());
		let target = session::normalize_original_uri(target.as_ref());
		let uri: Uri = target.parse().expect("normalized local redirect target");
		let path = uri.path();
		if path == self.redirect_uri.callback_path
			|| self.login.as_ref().is_some_and(|v| path == v.path)
			|| self.logout.as_ref().is_some_and(|v| path == v.path)
		{
			"/".into()
		} else {
			target
		}
	}

	pub(super) fn handle_logout(&self, req: &Request) -> Result<Option<PolicyResponse>, Error> {
		if !self
			.logout
			.as_ref()
			.is_some_and(|logout| req.uri().path() == logout.path)
		{
			return Ok(None);
		}
		// Require a same-origin form POST. An absent Origin is rejected too.
		let origin = url::Url::parse(&self.redirect_uri.redirect_uri)
			.map_err(|_| Error::InvalidCallback)?
			.origin()
			.ascii_serialization();
		if req.method() != Method::POST
			|| req
				.headers()
				.get(header::ORIGIN)
				.and_then(|v| v.to_str().ok())
				!= Some(origin.as_str())
		{
			let response = http::Response::builder()
				.status(StatusCode::FORBIDDEN)
				.body(Body::empty())
				.map_err(|e| Error::Config(e.to_string()))?;
			return Ok(Some(PolicyResponse::default().with_response(response)));
		}
		let mut cookies = vec![
			self
				.session
				.clear_cookie(&self.session.cookie_name, self.redirect_uri.https),
		];
		let prefix = format!("{}.", self.session.transaction_cookie_prefix);
		cookies.extend(
			crate::http::iter_request_cookies(req)
				.filter(|cookie| cookie.name().starts_with(&prefix))
				.map(|cookie| {
					self
						.session
						.clear_cookie(cookie.name(), self.redirect_uri.https)
				}),
		);
		let mut response = build_redirect_response(
			self
				.logout
				.as_ref()
				.and_then(|v| v.redirect.as_deref())
				.or_else(|| self.login.as_ref().and_then(|v| v.redirect.as_deref()))
				.unwrap_or("/"),
			&cookies,
		)?;
		*response.status_mut() = StatusCode::SEE_OTHER;
		response.headers_mut().insert(
			header::CACHE_CONTROL,
			http::HeaderValue::from_static("no-store"),
		);
		Ok(Some(PolicyResponse::default().with_response(response)))
	}
}
