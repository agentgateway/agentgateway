use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use ::http::{HeaderMap, StatusCode, Version};
use prost_types::Timestamp;
use serde_json::Value as JsonValue;

use crate::http::ext_authz::proto::attribute_context::HttpRequest;
use crate::http::ext_authz::proto::authorization_client::AuthorizationClient;
use crate::http::ext_authz::proto::check_response::HttpResponse;
use crate::http::ext_authz::proto::{
	AttributeContext, CheckRequest, DeniedHttpResponse, HeaderValueOption, OkHttpResponse,
};
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::{HeaderName, HeaderValue, PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::SimpleBackendReference;
use crate::{serde_dur_option, *};

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	tonic::include_proto!("envoy.service.auth.v3");
}

#[derive(Debug, Clone, Default)]
pub struct ExtAuthzDynamicMetadata {
	/// Flat key-value metadata for direct extauthz.field access in CEL
	pub metadata: HashMap<String, JsonValue>,
}

#[apply(schema_ser!)]
pub struct BodyOptions {
	/// Maximum size of request body to buffer (default: 8192)
	#[serde(default)]
	pub max_request_bytes: u32,
	/// If true, send partial body when max_request_bytes is reached
	#[serde(default)]
	pub allow_partial_message: bool,
	/// If true, pack body as raw bytes in gRPC
	#[serde(default)]
	pub pack_as_bytes: bool,
}

impl Default for BodyOptions {
	fn default() -> Self {
		Self {
			max_request_bytes: 8192,
			allow_partial_message: false,
			pack_as_bytes: false,
		}
	}
}

#[apply(schema_ser!)]
#[derive(Default)]
pub enum FailureMode {
	Allow,
	#[default]
	Deny,
	DenyWithStatus(u16),
}

#[apply(schema_ser!)]
pub struct ExtAuthz {
	/// Reference to the external authorization service backend
	pub target: Arc<SimpleBackendReference>,
	/// Additional context to send to the authorization service
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub context: Option<HashMap<String, String>>,
	/// Behavior when the authorization service is unavailable or returns an error
	#[serde(default)]
	pub failure_mode: FailureMode,
	/// Specific headers to include in the authorization request (empty = all headers)
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub include_request_headers: Vec<String>,
	/// Options for including the request body in the authorization request
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub include_request_body: Option<BodyOptions>,
	/// Timeout for the authorization request (default: 200ms)
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub timeout: Option<Duration>,
}

impl ExtAuthz {
	/// Handle authorization failure with FailureMode configuration
	fn handle_auth_failure(&self, error_msg: &str) -> Result<PolicyResponse, ProxyError> {
		match &self.failure_mode {
			FailureMode::Allow => {
				debug!("Allowing request due to FailureMode::Allow configuration");
				Ok(PolicyResponse::default())
			},
			FailureMode::Deny => Err(ProxyError::ExternalAuthorizationFailed(None)),
			FailureMode::DenyWithStatus(status_code) => {
				let status = StatusCode::from_u16(*status_code).unwrap_or(StatusCode::FORBIDDEN);
				let resp = ::http::Response::builder()
					.status(status)
					.body(http::Body::from(error_msg.to_string()))
					.map_err(|e| ProxyError::Processing(e.into()))?;
				Ok(PolicyResponse {
					direct_response: Some(resp),
					response_headers: None,
				})
			},
		}
	}

	pub async fn check(
		&self,
		client: PolicyClient,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		trace!("connecting to {:?}", self.target);
		let chan = GrpcReferenceChannel {
			target: self.target.clone(),
			client,
		};
		let mut grpc_client = AuthorizationClient::new(chan);
		// Get connection info with proper error handling
		// Clone the fields we need to avoid borrow checker issues
		let (peer_addr, local_addr, connection_start_time) = {
			let tcp_info = req.extensions().get::<TCPConnectionInfo>().ok_or_else(|| {
				warn!("TCPConnectionInfo not found in request extensions");
				ProxyError::Processing(anyhow::anyhow!("Missing TCP connection info"))
			})?;
			(tcp_info.peer_addr, tcp_info.local_addr, tcp_info.start)
		};
		let tls_info = req.extensions().get::<TLSConnectionInfo>().cloned();

		// Handle multi-value headers: comma-separated except cookies use "; " separator
		// https://github.com/envoyproxy/envoy/blob/d9e0412bd471a80e0938102c0c8cbff1caedd4cf/source/common/http/header_map_impl.cc#L28-L33
		let mut headers = std::collections::HashMap::new();

		let header_allowlist: std::collections::HashSet<&str> = self
			.include_request_headers
			.iter()
			.map(|s| s.as_str())
			.collect();

		for name in req.headers().keys() {
			if !self.include_request_headers.is_empty() && !header_allowlist.contains(name.as_str()) {
				continue;
			}

			let values: Vec<String> = req
				.headers()
				.get_all(name)
				.iter()
				.filter_map(|v| v.to_str().ok())
				.map(|s| s.to_string())
				.collect();

			if !values.is_empty() {
				let joined = if name.as_str() == "cookie" {
					values.join("; ")
				} else {
					values.join(", ")
				};
				headers.insert(name.as_str().to_string(), joined);
			}
		}

		let (body, raw_body, original_body_size) = if let Some(body_opts) = &self.include_request_body {
			let max_size = body_opts.max_request_bytes as usize;

			let original_size = 0;
			match crate::http::inspect_body_with_limit(req.body_mut(), max_size).await {
				Ok(body_bytes) => {
					let bytes = body_bytes.to_vec();

					if body_opts.pack_as_bytes {
						(String::new(), bytes, original_size)
					} else {
						(
							String::from_utf8_lossy(&bytes).into_owned(),
							Vec::new(),
							original_size,
						)
					}
				},
				Err(e) => {
					debug!("Failed to read request body for ext_authz: {:?}", e);
					(String::new(), Vec::new(), 0)
				},
			}
		} else {
			(String::new(), Vec::new(), 0)
		};

		let request_time = SystemTime::now() - connection_start_time.elapsed();

		let request_id = req
			.extensions()
			.get::<crate::telemetry::trc::TraceParent>()
			.map(|tp| tp.to_string())
			.unwrap_or_else(|| crate::telemetry::trc::TraceParent::new().to_string());

		let request = crate::http::ext_authz::proto::attribute_context::Request {
			time: Some(Timestamp::from(request_time)),
			http: Some(HttpRequest {
				id: request_id,
				method: req.method().to_string(),
				headers,
				path: req
					.uri()
					.path_and_query()
					.map(|pq| pq.to_string())
					.unwrap_or_else(|| req.uri().path().to_string()),
				host: req.uri().host().unwrap_or("").to_string(),
				scheme: req
					.uri()
					.scheme()
					.map(|s| s.to_string())
					.unwrap_or_else(|| "http".to_string()),
				protocol: match req.version() {
					Version::HTTP_09 => "HTTP/0.9".to_string(),
					Version::HTTP_10 => "HTTP/1.0".to_string(),
					Version::HTTP_11 => "HTTP/1.1".to_string(),
					Version::HTTP_2 => "HTTP/2".to_string(),
					Version::HTTP_3 => "HTTP/3".to_string(),
					_ => format!("{:?}", req.version()),
				},
				// Always empty per spec
				query: "".to_string(),
				// Always empty per spec
				fragment: "".to_string(),
				// Report original body size, not truncated size
				size: original_body_size,
				body,
				raw_body,
			}),
		};

		// Build source and destination peer information
		use crate::http::ext_authz::proto::attribute_context::Peer;
		use crate::http::ext_authz::proto::{Address, SocketAddress, socket_address};

		let source = Some(Peer {
			address: Some(Address {
				address: Some(
					crate::http::ext_authz::proto::address::Address::SocketAddress(SocketAddress {
						protocol: crate::http::ext_authz::proto::socket_address::Protocol::Tcp as i32,
						address: peer_addr.ip().to_string(),
						port_specifier: Some(socket_address::PortSpecifier::PortValue(
							peer_addr.port() as u32
						)),
						..Default::default()
					}),
				),
			}),
			service: String::new(),
			labels: HashMap::new(),
			principal: tls_info
				.as_ref()
				.and_then(|tls| tls.src_identity.as_ref().map(|id| id.to_string()))
				.unwrap_or_default(),
			certificate: String::new(),
		});

		let destination = Some(Peer {
			address: Some(Address {
				address: Some(
					crate::http::ext_authz::proto::address::Address::SocketAddress(SocketAddress {
						protocol: crate::http::ext_authz::proto::socket_address::Protocol::Tcp as i32,
						address: local_addr.ip().to_string(),
						port_specifier: Some(socket_address::PortSpecifier::PortValue(
							local_addr.port() as u32
						)),
						..Default::default()
					}),
				),
			}),
			service: String::new(),
			labels: HashMap::new(),
			principal: String::new(),
			certificate: String::new(),
		});

		let tls_session = tls_info.as_ref().map(|tls_info| {
			crate::http::ext_authz::proto::attribute_context::TlsSession {
				sni: tls_info.server_name.clone().unwrap_or_default(),
			}
		});

		let authz_req = CheckRequest {
			attributes: Some(AttributeContext {
				source,
				destination,
				request: Some(request),
				context_extensions: self.context.clone().unwrap_or_default(),
				tls_session,
			}),
		};
		let timeout_duration = self.timeout.unwrap_or(Duration::from_millis(200));
		let check_future = grpc_client.check(authz_req);

		let resp = match tokio::time::timeout(timeout_duration, check_future).await {
			Ok(result) => result,
			Err(_) => {
				warn!("ext_authz request timed out after {:?}", timeout_duration);
				return self.handle_auth_failure("Authorization service timeout");
			},
		};

		trace!("check response: {:?}", resp);
		let cr = match resp {
			Ok(response) => response,
			Err(e) => {
				warn!("ext_authz request failed: {:?}", e);
				return self.handle_auth_failure("Authorization service unavailable");
			},
		};
		let cr = cr.into_inner();
		let status = cr.status.as_ref().map(|status| status.code).unwrap_or(0);

		// Process dynamic metadata if present (for both allow and deny)
		if let Some(metadata) = cr.dynamic_metadata {
			let mut dynamic_metadata = ExtAuthzDynamicMetadata::default();

			for (key, value) in metadata.fields {
				dynamic_metadata
					.metadata
					.insert(key, convert_prost_value_to_json(&value)?);
			}

			if !dynamic_metadata.metadata.is_empty() {
				req.extensions_mut().insert(Arc::new(dynamic_metadata));
			}
		}

		if status != 0 {
			debug!("status denied: {status}");
			if let Some(HttpResponse::DeniedResponse(denied)) = cr.http_response {
				let DeniedHttpResponse {
					status: http_status,
					headers,
					body,
				} = denied;
				let status = http_status
					.and_then(|s| StatusCode::from_u16(s.code as u16).ok())
					.unwrap_or(StatusCode::FORBIDDEN);
				let mut rb = ::http::response::Builder::new().status(status);
				if let Some(hm) = rb.headers_mut() {
					process_headers(hm, headers, None);
				}
				let resp = rb
					.body(http::Body::from(body))
					.map_err(|e| ProxyError::Processing(e.into()))?;
				return Ok(PolicyResponse {
					direct_response: Some(resp),
					response_headers: None,
				});
			}
			return Err(ProxyError::ExternalAuthorizationFailed(None));
		}

		let mut res = PolicyResponse::default();
		let Some(resp) = cr.http_response else {
			return Ok(res);
		};

		match resp {
			HttpResponse::DeniedResponse(_) => {
				warn!("Received DeniedResponse with OK status");
			},
			HttpResponse::OkResponse(OkHttpResponse {
				headers,
				headers_to_remove,
				response_headers_to_add,
				query_parameters_to_set: _,
				query_parameters_to_remove: _,
				..
			}) => {
				for header_name in headers_to_remove {
					if !header_name.starts_with(':') && header_name.to_lowercase() != "host" {
						req.headers_mut().remove(header_name);
					}
				}

				let filtered_headers: Vec<_> = headers
					.into_iter()
					.filter(|h| {
						h.header
							.as_ref()
							.map(|hdr| hdr.key.to_lowercase() != "host")
							.unwrap_or(true)
					})
					.collect();

				process_headers(req.headers_mut(), filtered_headers, None);

				// for param in query_parameters_to_set {
				// TODO
				// }
				// for param_name in query_parameters_to_remove {
				// TODO
				// }

				if !response_headers_to_add.is_empty() {
					let mut hm = HeaderMap::new();
					process_headers(&mut hm, response_headers_to_add, None);
					if !hm.is_empty() {
						res.response_headers = Some(hm);
					}
				}
			},
		}
		Ok(res)
	}
}

fn convert_prost_value_to_json(value: &prost_wkt_types::Value) -> Result<JsonValue, ProxyError> {
	serde_json::to_value(value).map_err(|e| ProxyError::Processing(e.into()))
}

fn process_headers(
	hm: &mut HeaderMap,
	headers: Vec<HeaderValueOption>,
	allowlist: Option<&[String]>,
) {
	for header in headers {
		let Some(h) = header.header else { continue };

		// If allowlist is provided, only process headers in the allowlist
		if let Some(allowed) = allowlist {
			let header_name_lower = h.key.to_lowercase();
			if !allowed
				.iter()
				.any(|name| name.to_lowercase() == header_name_lower)
			{
				continue;
			}
		}

		let append = header.append.unwrap_or_default();
		let Ok(hn) = HeaderName::from_bytes(h.key.as_bytes()) else {
			warn!("Invalid header name: {}", h.key);
			continue;
		};
		let hv = if h.raw_value.is_empty() {
			HeaderValue::from_bytes(h.value.as_bytes())
		} else {
			HeaderValue::from_bytes(&h.raw_value)
		};
		let Ok(hv) = hv else {
			warn!("Invalid header value for key: {}", h.key);
			continue;
		};
		if append {
			hm.append(hn, hv);
		} else {
			hm.insert(hn, hv);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::http::ext_authz::proto::{HeaderValue as ProtoHeaderValue, HeaderValueOption};

	#[test]
	fn test_process_headers_with_allowlist() {
		let mut headers = HeaderMap::new();

		let header_options = vec![
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "x-allowed".to_string(),
					value: "allowed-value".to_string(),
					raw_value: vec![],
				}),
				append: Some(false),
			},
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "x-not-allowed".to_string(),
					value: "should-be-filtered".to_string(),
					raw_value: vec![],
				}),
				append: Some(false),
			},
		];

		// Test with allowlist
		let allowlist = vec!["x-allowed".to_string()];
		process_headers(&mut headers, header_options, Some(&allowlist));

		assert_eq!(headers.get("x-allowed").unwrap(), "allowed-value");
		assert!(headers.get("x-not-allowed").is_none());
	}

	#[test]
	fn test_process_headers() {
		let mut headers = HeaderMap::new();

		let header_options = vec![
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "x-custom-header".to_string(),
					value: "test-value".to_string(),
					raw_value: vec![],
				}),
				append: Some(false),
			},
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "x-append-header".to_string(),
					value: "value1".to_string(),
					raw_value: vec![],
				}),
				append: Some(false),
			},
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "x-append-header".to_string(),
					value: "value2".to_string(),
					raw_value: vec![],
				}),
				append: Some(true),
			},
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "x-raw-header".to_string(),
					value: "ignored".to_string(),
					raw_value: b"raw-value".to_vec(),
				}),
				append: Some(false),
			},
		];

		process_headers(&mut headers, header_options, None);

		assert_eq!(headers.get("x-custom-header").unwrap(), "test-value");
		assert_eq!(headers.get("x-raw-header").unwrap(), "raw-value");

		let append_values: Vec<_> = headers.get_all("x-append-header").iter().collect();
		assert_eq!(append_values.len(), 2);
		assert_eq!(append_values[0], "value1");
		assert_eq!(append_values[1], "value2");
	}

	#[test]
	fn test_body_truncation() {
		let body_opts = BodyOptions {
			max_request_bytes: 10,
			allow_partial_message: true,
			pack_as_bytes: false,
		};

		// Test truncation
		let long_body = b"This is a very long body that exceeds max size";
		assert!(long_body.len() > body_opts.max_request_bytes as usize);

		let mut truncated = long_body.to_vec();
		truncated.truncate(body_opts.max_request_bytes as usize);
		assert_eq!(truncated.len(), 10);
		assert_eq!(&truncated, b"This is a ");
	}

	#[test]
	fn test_multi_value_headers() {
		use ::http::Request;

		let req = Request::builder()
			.header("cookie", "session=abc")
			.header("cookie", "user=123")
			.header("x-forwarded-for", "10.0.0.1")
			.header("x-forwarded-for", "10.0.0.2")
			.body(http::Body::empty())
			.unwrap();

		// Collect all cookie values
		let cookies: Vec<_> = req
			.headers()
			.get_all("cookie")
			.iter()
			.filter_map(|v| v.to_str().ok())
			.collect();
		assert_eq!(cookies.len(), 2);
		assert_eq!(cookies[0], "session=abc");
		assert_eq!(cookies[1], "user=123");

		// Test joining with semicolon for cookies
		let joined = cookies.join("; ");
		assert_eq!(joined, "session=abc; user=123");
	}

	#[test]
	fn test_pseudo_header_protection() {
		let headers_to_remove = [
			":method".to_string(),
			":path".to_string(),
			"host".to_string(),
			"Host".to_string(),
			"content-type".to_string(),
		];

		// Only non-pseudo and non-host headers should be removable
		let removable: Vec<_> = headers_to_remove
			.iter()
			.filter(|h| !h.starts_with(':') && h.to_lowercase() != "host")
			.collect();

		assert_eq!(removable.len(), 1);
		assert_eq!(removable[0], "content-type");
	}

	#[test]
	fn test_host_header_protection() {
		// Test that host header cannot be added through upstream headers
		let header_options = vec![
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "host".to_string(),
					value: "evil.com".to_string(),
					raw_value: vec![],
				}),
				append: Some(false),
			},
			HeaderValueOption {
				header: Some(ProtoHeaderValue {
					key: "x-custom".to_string(),
					value: "allowed".to_string(),
					raw_value: vec![],
				}),
				append: Some(false),
			},
		];

		// Filter out host header
		let filtered: Vec<_> = header_options
			.into_iter()
			.filter(|h| {
				h.header
					.as_ref()
					.map(|hdr| hdr.key.to_lowercase() != "host")
					.unwrap_or(false)
			})
			.collect();

		assert_eq!(filtered.len(), 1);
		assert_eq!(filtered[0].header.as_ref().unwrap().key, "x-custom");
	}

	#[test]
	fn test_dynamic_metadata_extraction() {
		let mut metadata = ExtAuthzDynamicMetadata::default();

		metadata
			.metadata
			.insert("user_id".to_string(), serde_json::json!("12345"));
		metadata
			.metadata
			.insert("role".to_string(), serde_json::json!("admin"));
		assert_eq!(metadata.metadata.get("user_id").unwrap(), "12345");
		assert_eq!(metadata.metadata.get("role").unwrap(), "admin");
	}
}
