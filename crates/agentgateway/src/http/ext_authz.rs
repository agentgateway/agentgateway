use std::borrow::Borrow;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use ::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use prost_types::Timestamp;
use serde_json::Value as JsonValue;

use crate::cel::{BufferedBody, Expression, Value};
use crate::http::ext_authz::proto::attribute_context::HttpRequest;
use crate::http::ext_authz::proto::authorization_client::AuthorizationClient;
use crate::http::ext_authz::proto::check_response::HttpResponse;
use crate::http::ext_authz::proto::{
	AttributeContext, CheckRequest, DeniedHttpResponse, HeaderValueOption, Metadata, OkHttpResponse,
};
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::filters::BackendRequestTimeout;
use crate::http::{
	HeaderName, HeaderOrPseudo, PolicyResponse, Request, RequestOrResponse, Response,
	envoy_proto_common, jwt,
};
use crate::proxy::dtrace::{Severity, pol_event, pol_result_timed};
use crate::proxy::httpproxy::PolicyClient;
use crate::proxy::{ProxyError, ProxyResponse, dtrace};
use crate::transport::stream::{TCPConnectionInfo, TLSConnectionInfo};
use crate::types::agent::{BackendTrafficPolicy, SimpleBackendReference};
use crate::*;

const TRACE_POLICY_KIND: &str = "ext_auth";

#[cfg(test)]
#[path = "ext_authz_tests.rs"]
mod tests;

#[allow(warnings)]
#[allow(clippy::derive_partial_eq_without_eq)]
pub mod proto {
	pub use protos::envoy::service::auth::v3::*;
	pub use protos::envoy::service::common::v3::{
		HeaderValue, HeaderValueOption, HttpStatus, Metadata, Status, StatusCode, header_value_option,
	};
}

#[apply(schema!)]
#[derive(Default, ::cel::DynamicType)]
pub struct ExtAuthzDynamicMetadata(serde_json::Map<String, JsonValue>);

#[apply(schema!)]
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

#[apply(schema!)]
#[derive(Default)]
pub enum FailureMode {
	Allow,
	#[default]
	Deny,
	DenyWithStatus(u16),
}

#[apply(schema!)]
pub enum Protocol {
	#[serde(rename_all = "camelCase")]
	Grpc {
		/// Additional context to send to the authorization service.
		/// This maps to the `context_extensions` field of the request, and only allows static values.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		context: Option<HashMap<String, String>>,
		/// Additional metadata to send to the authorization service.
		/// This maps to the `metadata_context.filter_metadata` field of the request, and allows dynamic CEL expressions.
		/// If unset, by default the `envoy.filters.http.jwt_authn` key is set if the JWT policy is used as well, for compatibility.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		metadata: Option<HashMap<String, Arc<cel::Expression>>>,
	},
	#[serde(rename_all = "camelCase")]
	Http {
		path: Option<Arc<cel::Expression>>,
		/// When using the HTTP protocol, and the server returns unauthorized, redirect to the URL resolved by
		/// the provided expression rather than directly returning the error.
		redirect: Option<Arc<cel::Expression>>,
		/// Specific headers from the authorization response will be copied into the request to the backend.
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		#[serde_as(as = "Vec<crate::serdes::SerAsStr>")]
		#[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
		include_response_headers: Vec<HeaderName>,
		/// Specific headers to add in the authorization request (empty = all headers), based on the expression
		#[serde(default, skip_serializing_if = "HashMap::is_empty")]
		add_request_headers: HashMap<HeaderOrPseudo, Arc<cel::Expression>>,
		/// Metadata to include under the `extauthz` variable, based on the authorization response.
		#[serde(default, skip_serializing_if = "HashMap::is_empty")]
		metadata: HashMap<String, Arc<cel::Expression>>,
	},
}

impl Default for Protocol {
	fn default() -> Self {
		Protocol::Grpc {
			context: None,
			metadata: None,
		}
	}
}

#[apply(schema!)]
pub struct ExtAuthz {
	/// Reference to the external authorization service backend
	#[serde(flatten)]
	pub target: Arc<SimpleBackendReference>,
	/// Policies to connect to the backend
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde(deserialize_with = "crate::types::local::de_from_local_backend_policy")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "Option<crate::types::local::SimpleLocalBackendPolicies>")
	)]
	pub policies: Vec<BackendTrafficPolicy>,
	/// The ext_authz protocol to use. Unless you need to integrate with an HTTP-only server, gRPC is recommended.
	#[serde(default)]
	pub protocol: Protocol,
	/// Behavior when the authorization service is unavailable or returns an error
	#[serde(default)]
	pub failure_mode: FailureMode,
	/// Specific headers to include in the authorization request.
	/// If unset, the gRPC protocol sends all request headers. The HTTP protocol sends only 'Authorization'.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub include_request_headers: Vec<HeaderOrPseudo>,
	/// Options for including the request body in the authorization request
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub include_request_body: Option<BodyOptions>,
}

impl ExtAuthz {
	async fn buffer_request_body(
		req: &mut Request,
		body_opts: &BodyOptions,
	) -> Result<BufferedRequestBody, BufferRequestBodyError> {
		let max_size = body_opts.max_request_bytes as usize;

		let peek_limit = max_size.saturating_add(1);
		let body = crate::http::inspect_body_with_limit(req.body_mut(), peek_limit)
			.await
			.map_err(BufferRequestBodyError::Read)?;
		let is_partial = body.len() > max_size;

		if is_partial && !body_opts.allow_partial_message {
			return Err(BufferRequestBodyError::TooLarge);
		}

		let body = if is_partial {
			body.slice(0..max_size)
		} else {
			body
		};
		let original_size = match is_partial {
			false => i64::try_from(body.len()).unwrap_or(i64::MAX),
			true => -1,
		};

		Ok(BufferedRequestBody {
			body,
			is_partial,
			original_size,
		})
	}

	/// Handle authorization failure with FailureMode configuration
	fn handle_auth_failure(&self, error_msg: &str) -> Result<PolicyResponse, ProxyError> {
		match &self.failure_mode {
			FailureMode::Allow => {
				dtrace::pol_event!(
					Severity::Info,
					"Allowing request due to FailureMode::Allow configuration"
				);
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

	fn get_header_values(
		&self,
		req: &Request,
		name: &HeaderName,
		headers: &mut HashMap<String, String>,
	) {
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

	pub async fn check(
		&self,
		client: PolicyClient,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		if matches!(self.protocol, Protocol::Http { .. }) {
			trace!(protocol = "http", "connecting to {:?}", self.target);
			return self.check_http(client, req).await;
		}
		let start = dtrace::timed_start();
		trace!(protocol = "grpc", "connecting to {:?}", self.target);

		let Protocol::Grpc { context, metadata } = &self.protocol else {
			unreachable!();
		};
		let chan = GrpcReferenceChannel {
			target: self.target.clone(),
			policies: Arc::new(self.policies.clone()),
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
		let pseudo_headers = crate::http::get_request_pseudo_headers(req);

		if self.include_request_headers.is_empty() {
			// Envoy includes pseudo-headers, so we do too.
			for (pseudo, value) in &pseudo_headers {
				headers.insert(pseudo.to_string(), value.clone());
			}
			for name in req.headers().keys() {
				self.get_header_values(req, name, &mut headers);
			}
		} else {
			// Only include requested headers (both regular and pseudo headers)
			for header_spec in &self.include_request_headers {
				match header_spec {
					HeaderOrPseudo::Header(header_name) => {
						self.get_header_values(req, header_name, &mut headers);
					},
					pseudo => {
						if let Some((_, value)) = pseudo_headers.iter().find(|(p, _)| p == pseudo) {
							headers.insert(header_spec.to_string(), value.clone());
						}
					},
				}
			}
		}

		let (body, raw_body, original_body_size) = if let Some(body_opts) = &self.include_request_body {
			match Self::buffer_request_body(req, body_opts).await {
				Ok(buffered) => {
					let bytes = buffered.body;
					if body_opts.pack_as_bytes {
						(String::new(), bytes.to_vec(), buffered.original_size)
					} else {
						(
							String::from_utf8_lossy(bytes.as_ref()).into_owned(),
							Vec::new(),
							buffered.original_size,
						)
					}
				},
				Err(BufferRequestBodyError::TooLarge) => {
					return Err(ProxyError::ExternalAuthorizationFailed(Some(
						StatusCode::PAYLOAD_TOO_LARGE,
					)));
				},
				Err(BufferRequestBodyError::Read(e)) => return Err(ProxyError::Processing(e)),
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
				host: req
					.uri()
					.authority()
					.map(|s| s.as_str())
					.unwrap_or("")
					.to_string(),
				scheme: req
					.uri()
					.scheme()
					.map(|s| s.to_string())
					.unwrap_or_else(|| "http".to_string()),
				protocol: http::version_str(&req.version()).to_string(),
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
				.and_then(|tls| {
					tls
						.src_identity
						.as_ref()
						.and_then(|id| id.identity.as_ref().map(|s| s.to_string()))
				})
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
				metadata_context: self.build_metadata(metadata, req)?,
				context_extensions: context.clone().unwrap_or_default(),
				tls_session,
			}),
		};

		let scope = dtrace::start_scope("ext_authz");
		let resp = grpc_client.check(authz_req).await;
		drop(scope);

		trace!("check response: {:?}", resp);
		let cr = match resp {
			Ok(response) => response,
			Err(e) => {
				warn!("ext_authz request failed: {}", e);
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
					.0
					.insert(key, envoy_proto_common::prost_value_to_json(&value)?);
			}

			if !dynamic_metadata.0.is_empty() {
				req.extensions_mut().insert(dynamic_metadata);
			}
		}

		if status != 0 {
			pol_result_timed!(start, Severity::Error, Apply, "denied: {status}");
			if let Some(HttpResponse::DeniedResponse(denied)) = cr.http_response {
				let DeniedHttpResponse {
					status: http_status,
					headers,
					body,
				} = denied;
				let status = http_status
					.and_then(|s| StatusCode::from_u16(s.code as u16).ok())
					.unwrap_or(StatusCode::FORBIDDEN);
				let rb = ::http::response::Builder::new().status(status);
				let mut resp = rb
					.body(http::Body::from(body))
					.map_err(|e| ProxyError::Processing(e.into()))?;
				process_headers(RequestOrResponse::Response(&mut resp), headers, None);
				return Ok(PolicyResponse {
					direct_response: Some(resp),
					response_headers: None,
				});
			}
			return Err(ProxyError::ExternalAuthorizationFailed(None));
		}

		let mut res = PolicyResponse::default();
		pol_result_timed!(start, Severity::Info, Apply, "allowed");
		let Some(resp) = cr.http_response else {
			return Ok(res);
		};

		match resp {
			HttpResponse::DeniedResponse(_) => {
				pol_event!("Received DeniedResponse with OK status");
			},
			HttpResponse::OkResponse(OkHttpResponse {
				headers,
				headers_to_remove,
				response_headers_to_add,
				query_parameters_to_set,
				query_parameters_to_remove,
				..
			}) => {
				for header_name in headers_to_remove {
					if !header_name.starts_with(':') && header_name.to_lowercase() != "host" {
						req.headers_mut().remove(header_name);
					}
				}

				process_headers(req.into(), headers, None);

				apply_query_parameters_to_request(
					req,
					&query_parameters_to_set,
					&query_parameters_to_remove,
				)?;

				if !response_headers_to_add.is_empty() {
					let mut hm = HeaderMap::new();
					process_raw_headers(&mut hm, response_headers_to_add);
					if !hm.is_empty() {
						res.response_headers = Some(hm);
					}
				}
			},
		}
		Ok(res)
	}

	pub async fn check_http(
		&self,
		client: PolicyClient,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
		let Protocol::Http {
			redirect,
			include_response_headers,
			add_request_headers,
			path,
			metadata,
		} = &self.protocol
		else {
			unreachable!();
		};

		let (body, is_partial_body) = if let Some(body_opts) = &self.include_request_body {
			match Self::buffer_request_body(req, body_opts).await {
				Ok(buffered) => (buffered.body, buffered.is_partial),
				Err(BufferRequestBodyError::TooLarge) => {
					return Err(ProxyError::ExternalAuthorizationFailed(Some(
						StatusCode::PAYLOAD_TOO_LARGE,
					)));
				},
				Err(BufferRequestBodyError::Read(e)) => return Err(ProxyError::Processing(e)),
			}
		} else {
			(Bytes::new(), false)
		};

		let path: Uri = match path {
			Some(path_expr) => {
				let exec = cel::Executor::new_request(req);
				let res = exec
					.eval(path_expr)
					.map_err(|e| anyhow::anyhow!("{e}"))
					.and_then(|v| {
						if let Value::String(s) = v {
							Ok(s)
						} else {
							Err(anyhow::anyhow!("redirect resolved to a non-string value"))
						}
					})
					.and_then(|v| {
						Uri::try_from(v.as_ref())
							.map_err(|e| anyhow::anyhow!("invalid uri {}: {e}", v.as_ref()))
					});
				match res {
					Ok(res) => res,
					Err(e) => {
						tracing::warn!("fail to evaluate path: {e}");
						return Err(ProxyError::ExternalAuthorizationFailed(None));
					},
				}
			},
			None => Uri::try_from(http::get_path_and_query(req.uri()))
				.map_err(|_| ProxyError::ExternalAuthorizationFailed(None))?,
		};

		// If the user defined their own path expression, use that.
		// Else, use the original URL path.
		let rb = ::http::Request::builder().method(req.method()).uri(path);
		let mut check_req = rb
			.body(http::Body::from(body))
			.map_err(|e| ProxyError::Processing(e.into()))?;

		// Include any request headers
		let include = if self.include_request_headers.is_empty() {
			&[HeaderOrPseudo::Header(http::header::AUTHORIZATION)]
		} else {
			self.include_request_headers.as_slice()
		};
		for h in include {
			if let Some(hv) = http::get_pseudo_or_header_value(h, req) {
				let value = http::HeaderOrPseudoValue::from_raw(h, hv.as_bytes());
				http::RequestOrResponse::Request(&mut check_req).apply_header(
					h,
					value,
					http::HeaderMutationAction::OverwriteIfExistsOrAdd,
				);
			}
		}
		if self.include_request_body.is_some() {
			check_req.headers_mut().insert(
				// Don't love this but its part of the "specification" so we follow it.
				HeaderName::from_static("x-envoy-auth-partial-body"),
				HeaderValue::from_static(if is_partial_body { "true" } else { "false" }),
			);
		}

		// Insert any headers derived from CEL expressions.
		for (hn, hv) in add_request_headers {
			let exec = cel::Executor::new_request(req);
			let res = exec.eval(hv).ok();
			let resv = http::HeaderOrPseudoValue::from_cel_result(hn, res);
			http::RequestOrResponse::Request(&mut check_req).apply_header(
				hn,
				resv,
				http::HeaderMutationAction::OverwriteIfExistsOrAdd,
			);
		}
		// Set the default request timeout. This can be overridden by a timeout on the Backend object itself.
		check_req
			.extensions_mut()
			.insert(BackendRequestTimeout(Duration::from_millis(200)));
		let scope = dtrace::start_scope("ext_authz");
		let resp = client.call_reference(check_req, &self.target).await;
		let mut resp = match resp {
			Ok(r) => r,
			Err(e) => {
				trace!("ext_authz failed {e}");
				return self.handle_auth_failure(&e.to_string());
			},
		};
		drop(scope);
		if resp.status().is_success() {
			for k in include_response_headers {
				if let Some(h) = resp.headers().get(k) {
					// TODO: today we always insert. We should consider adding a mode to append.
					req.headers_mut().insert(k.clone(), h.clone());
				}
			}
			if !metadata.is_empty() {
				if let Ok(body) = crate::http::inspect_response_body(&mut resp).await {
					resp.extensions_mut().insert(BufferedBody(body));
				};
				let m = metadata
					.iter()
					.filter_map(|(k, v)| match Self::eval_to_json(req, &resp, v) {
						Ok(r) => Some((k.to_string(), r)),
						Err(e) => {
							trace!("failed to evaluate: {e}");
							error!("failed to evaluate: {e}");
							None
						},
					})
					.collect::<serde_json::Map<_, _>>();
				req.extensions_mut().insert(ExtAuthzDynamicMetadata(m));
			}
			return Ok(PolicyResponse::default());
		}
		if (resp.status() == StatusCode::FORBIDDEN || resp.status() == StatusCode::UNAUTHORIZED)
			&& let Some(redir) = &redirect
		{
			let exec = cel::Executor::new_request(req);
			let s = exec
				.eval(redir)
				.map_err(|e| anyhow::anyhow!("{e}"))
				.and_then(|v| {
					if let Value::String(s) = v {
						Ok(s)
					} else {
						Err(anyhow::anyhow!("redirect resolved to a non-string value"))
					}
				});
			return match s {
				Err(e) => {
					tracing::warn!("fail to evaluate redirect: {e}");
					Err(ProxyError::ExternalAuthorizationFailed(None))
				},
				Ok(redir) => {
					let status = StatusCode::FOUND;
					let resp = ::http::Response::builder()
						.status(status)
						.header(header::LOCATION, redir.as_ref())
						.body(http::Body::empty())
						.map_err(|e| ProxyError::Processing(e.into()))?;
					Ok(PolicyResponse {
						direct_response: Some(resp),
						response_headers: None,
					})
				},
			};
		}
		trace!("ext_authz failed with code {}", resp.status());
		Ok(PolicyResponse {
			direct_response: Some(resp),
			response_headers: None,
		})
	}

	fn build_metadata(
		&self,
		metadata: &Option<HashMap<String, Arc<cel::Expression>>>,
		req: &mut Request,
	) -> Result<Option<Metadata>, ProxyError> {
		Ok(match &metadata {
			Some(meta) => {
				let m = meta
					.iter()
					.filter_map(|(k, v)| match Self::eval_to_pb(req, v) {
						Ok(r) => Some((k.to_string(), r)),
						Err(e) => {
							trace!("failed to evaluate: {e}");
							None
						},
					})
					.collect();
				Some(Metadata { filter_metadata: m })
			},
			None => {
				if let Some(jc) = req.extensions().get::<jwt::Claims>() {
					Some(Metadata {
						filter_metadata: HashMap::from([(
							"envoy.filters.http.jwt_authn".to_string(),
							envoy_proto_common::json_to_struct(
								serde_json::json!({"jwt_payload": jc.inner.clone()}),
							)?,
						)]),
					})
				} else {
					// Envoy always set this, even if there is no metadata, so do the same for compatibility.
					Some(Metadata {
						filter_metadata: HashMap::new(),
					})
				}
			},
		})
	}

	fn eval_to_pb(req: &Request, v: &Expression) -> anyhow::Result<prost_wkt_types::Struct> {
		let exec = cel::Executor::new_request(req);
		let res = exec.eval(v)?;
		let js = res.json().map_err(|_| cel::Error::JsonConvert)?;
		let pb = envoy_proto_common::json_to_struct(js)?;
		Ok(pb)
	}

	fn eval_to_json(
		req: &Request,
		resp: &Response,
		v: &Expression,
	) -> anyhow::Result<serde_json::Value> {
		let exec = cel::Executor::new_request_and_response(req, resp);
		let res = exec.eval(v)?;
		let js = res.json().map_err(|_| cel::Error::JsonConvert)?;
		Ok(js)
	}
}

impl crate::store::RequestPolicyTrait for ExtAuthz {
	async fn apply(
		&self,
		client: &PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyResponse> {
		self
			.check(client.clone(), req)
			.await
			.map_err(ProxyResponse::from)
	}

	fn expressions(&self) -> impl Iterator<Item = &cel::Expression> {
		let iter: Box<dyn Iterator<Item = &cel::Expression>> = match &self.protocol {
			Protocol::Grpc {
				metadata: Some(m), ..
			} => Box::new(m.values().map(|v| v.as_ref())),
			Protocol::Http {
				redirect,
				path,
				add_request_headers,
				// TODO: this runs on the response. We would ideally have a way to NOT consider the response
				// attributes from this.
				metadata: m,
				..
			} => Box::new(
				add_request_headers
					.values()
					.map(|v| v.as_ref())
					.chain(m.values().map(|v| v.as_ref()))
					.chain(redirect.as_deref())
					.chain(path.as_deref()),
			),
			_ => Box::new(std::iter::empty()),
		};
		iter
	}
}

struct BufferedRequestBody {
	body: Bytes,
	is_partial: bool,
	original_size: i64,
}

#[derive(Debug)]
enum BufferRequestBodyError {
	TooLarge,
	Read(anyhow::Error),
}

fn apply_query_parameters_to_request(
	req: &mut Request,
	query_parameters_to_set: &[proto::QueryParameter],
	query_parameters_to_remove: &[String],
) -> Result<(), ProxyError> {
	crate::http::modify_query_parameters(
		req.uri_mut(),
		query_parameters_to_set
			.iter()
			.map(|param| (param.key.as_str(), param.value.as_str())),
		query_parameters_to_remove.iter().map(String::as_str),
	)
	.map_err(ProxyError::Processing)
}

fn process_headers(
	mut rr: http::RequestOrResponse,
	headers: Vec<HeaderValueOption>,
	allowlist: Option<&[String]>,
) {
	for header in headers {
		let header = header.borrow();
		let Some(ref h) = header.header else { continue };

		// If allowlist is provided, only process headers in the allowlist
		let header_name_lower = h.key.to_lowercase();
		if let Some(allowed) = allowlist
			&& !allowed
				.iter()
				.any(|name| name.to_lowercase() == header_name_lower)
		{
			continue;
		}

		envoy_proto_common::apply_header_option(&mut rr, header)
	}
}

fn process_raw_headers(hm: &mut HeaderMap, headers: Vec<HeaderValueOption>) {
	for header in headers {
		let Some(ref h) = header.header else { continue };

		let Ok(hn) = HeaderName::from_bytes(h.key.as_bytes()) else {
			warn!("Invalid header name: {}", h.key);
			continue;
		};
		let _ = envoy_proto_common::apply_header_value_option(hm, &hn, &header);
	}
}
