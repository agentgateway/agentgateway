use bytes::Bytes;
use serde::de::Error;

use crate::*;

#[cfg(test)]
#[path = "buffering_tests.rs"]
mod tests;

#[apply(schema_ser_schema!)]
#[cfg_attr(feature = "schema", schemars(with = "BufferingSerde"))]
pub struct Buffering {
	pub buffer_request_body: bool,
}

impl<'de> serde::Deserialize<'de> for Buffering {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Buffering::try_from(BufferingSerde::deserialize(deserializer)?).map_err(D::Error::custom)
	}
}

#[apply(schema_de!)]
pub struct BufferingSerde {
	#[serde(default)]
	pub buffer_request_body: bool,
}

impl TryFrom<BufferingSerde> for Buffering {
	type Error = anyhow::Error;
	fn try_from(value: BufferingSerde) -> Result<Self, Self::Error> {
		Ok(Buffering {
			buffer_request_body: value.buffer_request_body,
		})
	}
}

/// Inserted into request extensions when the buffering policy has fully drained the request
/// body into memory. Other policies (CEL inspection, ext_proc, retry replay) can read this
/// to avoid redundant buffering of the same bytes.
#[derive(Clone, Debug)]
pub struct BufferedRequestBody(pub Bytes);

impl Buffering {
	/// Drains the request body into memory and stores the bytes in a `BufferedRequestBody`
	/// extension if the policy enables buffering. No-op when buffering is disabled, when the
	/// body has already been buffered, or for upgrade requests (whose "body" only exists
	/// post-handshake as the upgraded byte stream).
	pub async fn apply_to_request(
		&self,
		req: &mut crate::http::Request,
	) -> Result<(), crate::proxy::ProxyResponse> {
		if !self.buffer_request_body {
			return Ok(());
		}
		if req.extensions().get::<BufferedRequestBody>().is_some() {
			return Ok(());
		}
		if req.headers().contains_key(::http::header::UPGRADE) {
			return Ok(());
		}

		let limit = crate::http::buffer_limit(req);
		let body = std::mem::replace(req.body_mut(), crate::http::Body::empty());
		let bytes = crate::http::read_body_with_limit(body, limit)
			.await
			.map_err(crate::proxy::ProxyError::Body)?;
		*req.body_mut() = crate::http::Body::from(bytes.clone());
		req.extensions_mut().insert(BufferedRequestBody(bytes));

		Ok(())
	}
}

impl crate::store::RequestPolicyTrait for Buffering {
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
