use serde::de::Error;

use crate::*;

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod buffer_tests;

#[apply(schema!)]
#[derive(Default)]
#[cfg_attr(feature = "schema", schemars(with = "BufferSerde"))]
pub struct BufferBody {
	pub max_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct Buffer {
	pub request: BufferBody,
	pub response: BufferBody,
}

impl<'de> serde::Deserialize<'de> for Buffer {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Buffer::try_from(BufferSerde::deserialize(deserializer)?).map_err(D::Error::custom)
	}
}

impl serde::Serialize for Buffer {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		BufferSerde {
			request: self.request.clone(),
			response: self.response.clone(),
		}
		.serialize(serializer)
	}
}

#[apply(schema!)]
pub struct BufferSerde {
	#[serde(default)]
	pub request: BufferBody,
	#[serde(default)]
	pub response: BufferBody,
}

impl TryFrom<BufferSerde> for Buffer {
	type Error = anyhow::Error;
	fn try_from(value: BufferSerde) -> Result<Self, Self::Error> {
		Ok(Buffer {
			request: value.request,
			response: value.response,
		})
	}
}

impl Buffer {
	/// Drains the request body into memory and replaces it with a `Body::from(Bytes)` wrapper.
	/// No-op when buffering is disabled or for upgrade requests (whose "body" only exists
	/// post-handshake as the upgraded byte stream).
	pub async fn apply_to_request(
		&self,
		req: &mut crate::http::Request,
	) -> Result<(), crate::proxy::ProxyResponse> {
		if self.request.max_bytes == 0 {
			trace!("request buffering disabled");
			return Ok(());
		}
		if req.headers().contains_key(::http::header::UPGRADE) {
			debug!("skipping request buffer for upgrade request");
			return Ok(());
		}

		let limit = crate::http::buffer_limit(req);
		let body = std::mem::replace(req.body_mut(), crate::http::Body::empty());
		let bytes = match crate::http::read_body_with_limit(body, limit).await {
			Ok(b) => b,
			Err(e) => {
				warn!(limit, error = %e, "failed to buffer request body");
				let resp = ::http::Response::builder()
					.status(::http::StatusCode::PAYLOAD_TOO_LARGE)
					.body(crate::http::Body::empty())
					.expect("static response builds");
				return Err(crate::proxy::ProxyResponse::DirectResponse(Box::new(resp)));
			},
		};
		debug!(bytes = bytes.len(), "buffered request body");
		*req.body_mut() = crate::http::Body::from(bytes);

		Ok(())
	}

	/// Drains the response body into memory and replaces it with a `Body::from(Bytes)` wrapper.
	/// No-op when buffering is disabled or for protocol-switching (101) responses whose
	/// "body" is the upgraded byte stream.
	pub async fn apply_to_response(
		&self,
		resp: &mut crate::http::Response,
	) -> Result<(), crate::proxy::ProxyResponse> {
		if self.response.max_bytes == 0 {
			trace!("response buffering disabled");
			return Ok(());
		}
		if resp.status() == ::http::StatusCode::SWITCHING_PROTOCOLS {
			debug!("skipping response buffer for protocol-switching response");
			return Ok(());
		}

		let limit = crate::http::response_buffer_limit(resp);
		let body = std::mem::replace(resp.body_mut(), crate::http::Body::empty());
		let bytes = match crate::http::read_body_with_limit(body, limit).await {
			Ok(b) => b,
			Err(e) => {
				warn!(limit, error = %e, "failed to buffer response body");
				let err = ::http::Response::builder()
					.status(::http::StatusCode::BAD_GATEWAY)
					.body(crate::http::Body::empty())
					.expect("static response builds");
				return Err(crate::proxy::ProxyResponse::DirectResponse(Box::new(err)));
			},
		};
		debug!(bytes = bytes.len(), "buffered response body");
		*resp.body_mut() = crate::http::Body::from(bytes);

		Ok(())
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

