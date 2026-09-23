use crate::*;

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod buffer_tests;

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum FailureMode {
	// Return error if body is larger than maxBytes.
	#[default]
	FailClosed,
	// Continue streaming if body is larger than maxBytes.
	FailOpen,
}

#[apply(schema!)]
#[derive(Default)]
pub struct BufferBody {
	/// Maximum body size to buffer in bytes.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_bytes: Option<usize>,
	/// Behavior when the body exceeds maxBytes: failClosed (reject) or failOpen (continue).
	#[serde(default)]
	pub failure_mode: FailureMode,
}

#[apply(schema!)]
pub struct Buffer {
	/// Buffer incoming request bodies before forwarding.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub request: Option<BufferBody>,
	/// Buffer upstream response bodies before sending them to the client.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub response: Option<BufferBody>,
}

impl Buffer {
	/// Applies request body buffering before forwarding.
	/// No-op when buffering is disabled or for upgrade requests (whose "body" only exists
	/// post-handshake as the upgraded byte stream).
	pub async fn apply_to_request(
		&self,
		req: &mut crate::http::Request,
	) -> Result<(), crate::proxy::ProxyResponse> {
		let Some(request) = self.request.as_ref() else {
			trace!("request buffering disabled");
			return Ok(());
		};
		if req.headers().contains_key(::http::header::UPGRADE) {
			debug!("skipping request buffer for upgrade request");
			return Ok(());
		}

		let limit = request
			.max_bytes
			.unwrap_or_else(|| crate::http::buffer_limit(req));
		match buffer_body(req.body_mut(), limit, request.failure_mode).await {
			Ok(()) => {},
			Err(e) => {
				warn!(limit, error = %e, "failed to buffer request body");
				let resp = ::http::Response::builder()
					.status(::http::StatusCode::PAYLOAD_TOO_LARGE)
					.body(crate::http::Body::empty())
					.expect("static response builds");
				return Err(crate::proxy::ProxyResponse::DirectResponse(Box::new(resp)));
			},
		}
		// Preserve an unset limit so LLM processing can apply its own default later.
		if let Some(limit) = request.max_bytes {
			req
				.extensions_mut()
				.insert(crate::transport::BufferLimit::new(limit));
		}
		Ok(())
	}

	/// Applies response body buffering before sending it to the client.
	/// No-op when buffering is disabled or for protocol-switching (101) responses whose
	/// "body" is the upgraded byte stream.
	pub async fn apply_to_response(
		&self,
		resp: &mut crate::http::Response,
	) -> Result<(), crate::proxy::ProxyResponse> {
		let Some(response) = self.response.as_ref() else {
			trace!("response buffering disabled");
			return Ok(());
		};
		if resp.status() == ::http::StatusCode::SWITCHING_PROTOCOLS {
			debug!("skipping response buffer for protocol-switching response");
			return Ok(());
		}

		let limit = response
			.max_bytes
			.unwrap_or_else(|| crate::http::response_buffer_limit(resp));
		match buffer_body(resp.body_mut(), limit, response.failure_mode).await {
			Ok(()) => {},
			Err(e) => {
				warn!(limit, error = %e, "failed to buffer response body");
				let err = ::http::Response::builder()
					.status(::http::StatusCode::BAD_GATEWAY)
					.body(crate::http::Body::empty())
					.expect("static response builds");
				return Err(crate::proxy::ProxyResponse::DirectResponse(Box::new(err)));
			},
		}
		resp
			.extensions_mut()
			.insert(crate::transport::BufferLimit::new(limit));

		Ok(())
	}
}

// Buffers `body` up to `limit`, picking what to do on overflow.
//
// `FailClosed` fails (so the caller can send a 413/502) if the body is bigger than `limit`.
// `FailOpen` buffers up to `limit` and streams the rest.
async fn buffer_body(
	body: &mut crate::http::Body,
	limit: usize,
	failure_mode: FailureMode,
) -> anyhow::Result<()> {
	if failure_mode == FailureMode::FailOpen && limit == 0 {
		return Ok(());
	}
	match body.inspect(limit).await? {
		crate::http::BodyInspection::Complete(body) => {
			debug!(b = body.len(), "buffered body");
			Ok(())
		},
		crate::http::BodyInspection::Partial(_) if failure_mode == FailureMode::FailClosed => {
			anyhow::bail!("body exceeds buffer limit of {limit} bytes")
		},
		crate::http::BodyInspection::Partial(_) => {
			debug!(limit, "buffered up to limit, streaming the rest");
			Ok(())
		},
	}
}

impl crate::store::RequestPolicyTrait for Buffer {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		self.apply_to_request(req).await?;
		Ok(Default::default())
	}
}

impl crate::store::ResponsePolicyTrait for Buffer {
	async fn apply(
		&self,
		_log: &mut crate::telemetry::log::RequestLog,
		res: &mut crate::http::Response,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		self.apply_to_response(res).await?;
		Ok(Default::default())
	}
}
