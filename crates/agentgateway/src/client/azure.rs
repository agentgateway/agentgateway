use crate::client::{ApplicationTransport, Call, Client};
use crate::types::agent::Target;
use async_trait::async_trait;
use azure_core::error::ResultExt;
use futures_util::TryStreamExt;
use http_body_util::BodyExt;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use tracing::{debug, error, warn};
use typespec_client_core::http::{AsyncRawResponse, Sanitizer};

#[async_trait]
impl azure_core::http::HttpClient for Client {
	async fn execute_request(
		&self,
		request: &azure_core::http::Request,
	) -> azure_core::Result<AsyncRawResponse> {
		let url = request.url().clone();
		let method = request.method();
		let mut req = ::http::Request::builder();
		req = req.method(from_method(method)?).uri(url.as_str());
		for (name, value) in request.headers().iter() {
			req = req.header(name.as_str(), value.as_str());
		}
		let body = request.body().clone();

		let request = match body {
			azure_core::http::Body::Bytes(bytes) => req.body(crate::http::Body::from(bytes)),

			// We cannot currently implement `Body::SeekableStream` for WASM
			// because `reqwest::Body::wrap_stream()` is not implemented for WASM.
			#[cfg(not(target_arch = "wasm32"))]
			azure_core::http::Body::SeekableStream(seekable_stream) => {
				req.body(crate::http::Body::from_stream(seekable_stream))
			},
		}
		.map_err(|e| {
			azure_core::Error::with_error(
				azure_core::error::ErrorKind::Other,
				e,
				"failed to build `agentgateway::client::Client` request",
			)
		})?;

		debug!(
			"performing request {method} '{}' with `agentgateway::client::Client`",
			url.sanitize(&typespec_client_core::http::DEFAULT_ALLOWED_QUERY_PARAMETERS)
		);
		let rsp = self
			.call(Call {
				req: request,
				target: match url.host().expect("url must have a host") {
					url::Host::Domain(h) => Target::try_from((h, url.port_or_known_default().unwrap_or(80)))
						.map_err(|e| {
							azure_core::Error::with_error(
								azure_core::error::ErrorKind::Other,
								e,
								"failed to parse host for `agentgateway::client::Client` request",
							)
						})?,
					url::Host::Ipv4(ip) => Target::Address(SocketAddr::from((
						ip,
						url.port_or_known_default().unwrap_or(80),
					))),
					url::Host::Ipv6(ip) => Target::Address(SocketAddr::from((
						ip,
						url.port_or_known_default().unwrap_or(80),
					))),
				},
				transport: if url.scheme() == "https" {
					ApplicationTransport::Tls(crate::http::backendtls::SYSTEM_TRUST.base_config()).into()
				} else {
					ApplicationTransport::Plaintext.into()
				},
			})
			.await
			.map_err(|e| {
				error!("request failed: {e}");
				azure_core::Error::with_error(
					azure_core::error::ErrorKind::Io,
					e,
					"failed to execute `agentgateway::client::Client` request",
				)
			})?;

		let status = rsp.status();
		let headers = to_headers(rsp.headers());

		let body: azure_core::http::response::PinnedStream =
			Box::pin(rsp.into_data_stream().map_err(|error| {
				azure_core::Error::with_error(
					azure_core::error::ErrorKind::Io,
					error,
					"error converting `reqwest` request into a byte stream",
				)
			}));

		Ok(AsyncRawResponse::new(status.as_u16().into(), headers, body))
	}
}

fn from_method(method: azure_core::http::Method) -> azure_core::Result<http::Method> {
	match method {
		azure_core::http::Method::Get => Ok(http::Method::GET),
		azure_core::http::Method::Head => Ok(http::Method::HEAD),
		azure_core::http::Method::Post => Ok(http::Method::POST),
		azure_core::http::Method::Put => Ok(http::Method::PUT),
		azure_core::http::Method::Delete => Ok(http::Method::DELETE),
		azure_core::http::Method::Patch => Ok(http::Method::PATCH),
		_ => http::Method::from_str(method.as_str())
			.with_kind(azure_core::error::ErrorKind::DataConversion),
	}
}

fn to_headers(map: &::http::HeaderMap) -> azure_core::http::headers::Headers {
	let map = map
		.iter()
		.filter_map(|(k, v)| {
			let key = k.as_str();
			if let Ok(value) = v.to_str() {
				Some((
					azure_core::http::headers::HeaderName::from(key.to_owned()),
					azure_core::http::headers::HeaderValue::from(value.to_owned()),
				))
			} else {
				warn!("header value for `{key}` is not utf8");
				None
			}
		})
		.collect::<HashMap<_, _>>();
	azure_core::http::headers::Headers::from(map)
}
