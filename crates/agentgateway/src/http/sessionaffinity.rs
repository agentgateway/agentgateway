use std::sync::Arc;

use twox_hash::XxHash3_64;

use crate::cel::{Executor, Expression};
use crate::*;

/// Pins requests that carry the same affinity value to the same healthy service endpoint.
///
/// This policy is intentionally stateless. Every gateway replica computes the same rendezvous
/// hash from the request affinity value and the currently available endpoint set.
#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "SessionAffinityPolicy"))]
pub struct Policy {
	/// CEL expression evaluated against request state. It must return a string or bytes value.
	/// Examples: `request.headers["x-session-id"]` or `string(source.address)`.
	pub source: Arc<Expression>,
}

impl Policy {
	/// Returns a stable, non-secret affinity key suitable for endpoint selection.
	/// Evaluation errors, non-string/bytes values, and empty values disable affinity for that
	/// request (it falls back to normal load balancing). Each miss is logged at trace level so a
	/// misconfigured expression or a missing header can be diagnosed without failing the request.
	pub fn affinity_key(&self, req: &http::Request) -> Option<u64> {
		let executor = Executor::new_request(req);
		let value = match executor.eval(&self.source) {
			Ok(value) => value,
			Err(err) => {
				trace!(
					expression = %self.source.original_expression,
					error = %err,
					"session affinity: expression evaluation failed; falling back to load balancing"
				);
				return None;
			},
		};
		let value = value.always_materialize();
		let bytes = match value.as_bytes_pre_materialized() {
			Ok(bytes) => bytes,
			Err(_) => {
				trace!(
					expression = %self.source.original_expression,
					value_type = value.type_of().as_str(),
					"session affinity: expression did not return a string or bytes value; falling back to load balancing"
				);
				return None;
			},
		};
		if bytes.is_empty() {
			trace!(
				expression = %self.source.original_expression,
				"session affinity: expression returned an empty value; falling back to load balancing"
			);
			return None;
		}

		Some(XxHash3_64::oneshot(bytes))
	}

	pub fn register_expressions(&self, ctx: &mut crate::cel::ContextBuilder) {
		ctx.register_expression(self.source.as_ref());
	}
}

#[cfg(test)]
mod tests {
	use std::net::{IpAddr, Ipv4Addr, SocketAddr};

	use super::*;
	use crate::cel::SourceContext;
	use crate::transport::stream::TCPConnectionInfo;

	fn policy(source: &str) -> Policy {
		Policy {
			source: Arc::new(Expression::new_strict(source).unwrap()),
		}
	}

	#[test]
	fn header_affinity_uses_configured_header_value() {
		let policy = policy(r#"request.headers["x-session-id"]"#);
		let request = |value: &'static str| {
			let mut request = http::Request::new(http::Body::empty());
			request
				.headers_mut()
				.insert("x-session-id", http::HeaderValue::from_static(value));
			request
		};
		let first = request("session-a");
		let second = request("session-a");
		let different = request("session-b");

		assert_eq!(policy.affinity_key(&first), policy.affinity_key(&second));
		assert_ne!(policy.affinity_key(&first), policy.affinity_key(&different));
	}

	#[test]
	fn policy_deserializes_cel_source() {
		let policy: Policy = serde_yaml::from_str(
			r#"
source: request.headers["x-session-id"]
"#,
		)
		.unwrap();
		assert_eq!(
			policy.source.original_expression,
			r#"request.headers["x-session-id"]"#
		);
	}

	#[test]
	fn missing_header_falls_back_to_normal_load_balancing() {
		let policy = policy(r#"request.headers["x-session-id"]"#);
		let request = http::Request::new(http::Body::empty());
		assert_eq!(policy.affinity_key(&request), None);
	}

	#[test]
	fn empty_header_value_falls_back_to_normal_load_balancing() {
		let policy = policy(r#"request.headers["x-session-id"]"#);
		let mut request = http::Request::new(http::Body::empty());
		request
			.headers_mut()
			.insert("x-session-id", http::HeaderValue::from_static(""));
		assert_eq!(policy.affinity_key(&request), None);
	}

	#[test]
	fn client_ip_affinity_ignores_source_port() {
		fn request(port: u16) -> http::Request {
			let mut request = http::Request::new(http::Body::empty());
			let tcp = TCPConnectionInfo {
				peer_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)), port),
				local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 80),
				start: Instant::now(),
				raw_peer_addr: None,
			};
			request
				.extensions_mut()
				.insert(SourceContext::from_tcp_connection(&tcp, None, None));
			request
		}

		let policy = policy("string(source.address)");
		let first = policy
			.affinity_key(&request(10000))
			.expect("source context should produce an affinity key");
		let second = policy
			.affinity_key(&request(20000))
			.expect("source context should produce an affinity key");
		assert_eq!(first, second);
	}
}
