use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::cel::CelDuration;
use crate::telemetry::log::SpanWriter;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallLabels};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[dynamic(rename_all = "camelCase")]
pub struct UpstreamCall {
	/// Call category: `Primary` (backend request) or `Policy` (policy callout).
	pub kind: String,
	/// Call subtype: `Http`, `Llm`, `Mcp`, `ExtAuthz`, `Substrate`, `ExtProc`,
	/// `Guardrail`, `RateLimit`, `Oidc`, `Callout`, or a custom subtype name.
	pub subtype: String,
	/// MCP target name; connection address (host:port) for other calls.
	pub target: String,
	pub request: UpstreamRequest,
	pub response: UpstreamResponse,
	/// Time from before DNS resolution through response headers, including connection setup.
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub duration: Option<CelDuration>,
	/// Error description when the call fails. Exact wording may change between versions.
	pub error: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct UpstreamRequest {
	/// Outbound request headers after backend authentication, excluding tunnel proxy credentials.
	#[dynamic(with_value = "headers_to_value")]
	#[serde(with = "http_serde::header_map")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	pub headers: http::HeaderMap,
}

fn headers_to_value(headers: &http::HeaderMap) -> cel::Value<'_> {
	cel::Value::Dynamic(cel::types::dynamic::DynamicValue::new_owned(
		crate::cel::Headers::new(headers),
	))
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[dynamic(rename_all = "camelCase")]
pub struct UpstreamResponse {
	/// HTTP response status code; null when no HTTP response was received or for gRPC calls.
	pub code: Option<u16>,
	pub grpc_status: Option<i32>,
}

#[derive(Clone, Debug, Default)]
pub struct UpstreamCalls(Arc<Mutex<Vec<UpstreamCall>>>);

impl UpstreamCalls {
	pub fn take(&self) -> Vec<UpstreamCall> {
		std::mem::take(&mut *self.0.lock().unwrap())
	}
}

#[derive(Clone, Debug)]
struct OutboundRequest {
	calls: UpstreamCalls,
	labels: OutboundCallLabels,
}

#[derive(Clone, Debug)]
pub(crate) struct UpstreamTarget(pub String);

impl SpanWriter {
	pub(crate) fn record_outbound(&self, req: &mut crate::http::Request, labels: OutboundCallLabels) {
		// Only record calls within the request lifetime; detached mirrors can outlive its log.
		if matches!(labels.kind, OutboundCallKind::Mirror) {
			req.extensions_mut().remove::<OutboundRequest>();
			return;
		}
		if let Some(calls) = &self.upstream_calls {
			req.extensions_mut().insert(OutboundRequest {
				calls: calls.clone(),
				labels,
			});
		}
	}
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CapturedHeaders(Arc<Mutex<http::HeaderMap>>);

impl CapturedHeaders {
	pub fn update(req: &crate::http::Request) {
		if let Some(headers) = req.extensions().get::<Self>() {
			*headers.0.lock().unwrap() = req.headers().clone();
		}
	}
}

pub(crate) struct PendingCall {
	calls: UpstreamCalls,
	start: Instant,
	call: Option<UpstreamCall>,
	shared_headers: Option<CapturedHeaders>,
}

impl PendingCall {
	pub fn new(
		calls: UpstreamCalls,
		labels: OutboundCallLabels,
		target: String,
		headers: &http::HeaderMap,
	) -> Self {
		let mut this = Self {
			calls,
			shared_headers: None,
			start: Instant::now(),
			call: Some(UpstreamCall {
				kind: labels.kind.as_str().to_owned(),
				subtype: labels.subtype.as_str().to_owned(),
				target,
				request: UpstreamRequest::default(),
				response: UpstreamResponse::default(),
				duration: None,
				error: Some("request did not complete".to_owned()),
			}),
		};
		this.set_headers(headers);
		this
	}

	pub fn from_request(req: &crate::http::Request, target: impl ToString) -> Option<Self> {
		let outbound = req.extensions().get::<OutboundRequest>()?;
		let target = req
			.extensions()
			.get::<UpstreamTarget>()
			.map(|t| t.0.clone())
			.unwrap_or_else(|| target.to_string());
		Some(Self::new(
			outbound.calls.clone(),
			outbound.labels,
			target,
			req.headers(),
		))
	}

	pub fn share_headers(&mut self) -> CapturedHeaders {
		let headers = CapturedHeaders(Arc::new(Mutex::new(
			self.call.as_ref().unwrap().request.headers.clone(),
		)));
		self.shared_headers = Some(headers.clone());
		headers
	}

	pub fn set_headers(&mut self, headers: &http::HeaderMap) {
		self.call.as_mut().unwrap().request.headers = headers.clone();
	}

	pub fn set_status(&mut self, code: u16) {
		let call = self.call.as_mut().unwrap();
		call.response.code = Some(code);
		call.error = None;
	}

	pub fn set_grpc_status(&mut self, code: i32) {
		let call = self.call.as_mut().unwrap();
		call.response.grpc_status = Some(code);
		call.error = None;
	}

	pub fn set_error(&mut self, error: String) {
		self.call.as_mut().unwrap().error = Some(error);
	}
}

impl Drop for PendingCall {
	fn drop(&mut self) {
		if let Some(mut call) = self.call.take() {
			if let Some(headers) = &self.shared_headers {
				call.request.headers = std::mem::take(&mut *headers.0.lock().unwrap());
			}
			call.duration = CelDuration::from_std(self.start.elapsed());
			self.calls.0.lock().unwrap().push(call);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::cel::{Executor, Expression};
	use crate::http::Body;
	use crate::proxy::httpproxy::PolicyClient;
	use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
	use crate::test_helpers::proxymock::setup_proxy_test;

	#[test]
	fn mirrors_do_not_inherit_parent_recording() {
		let mut writer = SpanWriter::default();
		writer.upstream_calls = Some(UpstreamCalls::default());
		let mut parent = http::Request::new(Body::empty());
		writer.record_outbound(
			&mut parent,
			OutboundCallLabels {
				kind: OutboundCallKind::Primary,
				subtype: OutboundCallSubtype::Mcp,
			},
		);
		let mut mirror = http::Request::new(Body::empty());
		*mirror.extensions_mut() = parent.extensions().clone();
		writer.record_outbound(
			&mut mirror,
			OutboundCallLabels {
				kind: OutboundCallKind::Mirror,
				subtype: OutboundCallSubtype::Http,
			},
		);
		assert!(PendingCall::from_request(&mirror, "mirror").is_none());
		assert!(PendingCall::from_request(&parent, "parent").is_some());
	}

	#[tokio::test]
	async fn upstream_calls_exclude_plaintext_tunnel_credentials() {
		use crate::client::{ApplicationTransport, Call, Transport, TunnelConfig};
		let proxy = wiremock::MockServer::start().await;
		wiremock::Mock::given(wiremock::matchers::header(
			"proxy-authorization",
			"Bearer proxy-secret",
		))
		.respond_with(wiremock::ResponseTemplate::new(200))
		.expect(1)
		.mount(&proxy)
		.await;
		let calls = UpstreamCalls::default();
		let mut writer = SpanWriter::default();
		writer.upstream_calls = Some(calls.clone());
		let mut req = http::Request::builder()
			.uri("http://127.0.0.1:12345/resource")
			.header("authorization", "Bearer backend-secret")
			.body(Body::empty())
			.unwrap();
		writer.record_outbound(
			&mut req,
			OutboundCallLabels {
				kind: OutboundCallKind::Policy,
				subtype: OutboundCallSubtype::Oidc,
			},
		);
		// Exercise the shared capture used by gRPC as well as the HTTP record.
		let mut grpc = PendingCall::new(
			calls.clone(),
			OutboundCallLabels {
				kind: OutboundCallKind::Policy,
				subtype: OutboundCallSubtype::Oidc,
			},
			"grpc".to_owned(),
			req.headers(),
		);
		req.extensions_mut().insert(grpc.share_headers());
		let t = setup_proxy_test("{}").unwrap();
		let response = t
			.pi
			.upstream
			.call(Call {
				req,
				target: ("127.0.0.1", 12345).into(),
				connection: Transport::Tunnel(
					ApplicationTransport::Plaintext,
					TunnelConfig {
						target: crate::types::agent::Target::Address(*proxy.address()),
						connection: Box::new(Transport::Plain(ApplicationTransport::Plaintext).into()),
						token: Some("Bearer proxy-secret".parse().unwrap()),
						connect_headers: vec![],
						connect: false,
					},
				)
				.into(),
			})
			.await
			.unwrap();
		assert_eq!(response.status(), 200);
		drop(grpc);
		let records = calls.take();
		assert_eq!(records.len(), 2);
		for record in records {
			assert_eq!(
				record.request.headers["authorization"],
				"Bearer backend-secret"
			);
			assert!(!record.request.headers.contains_key("proxy-authorization"));
		}
	}

	#[tokio::test]
	async fn upstream_calls_capture_parallel_policy_requests_without_tracing() {
		let server = wiremock::MockServer::start().await;
		wiremock::Mock::given(wiremock::matchers::method("GET"))
			.respond_with(wiremock::ResponseTemplate::new(401))
			.mount(&server)
			.await;
		let calls = UpstreamCalls::default();
		let mut writer = SpanWriter::default();
		writer.upstream_calls = Some(calls.clone());
		let mut parent = http::Request::new(Body::empty());
		parent
			.headers_mut()
			.insert("authorization", "Bearer inbound".parse().unwrap());
		parent.extensions_mut().insert(writer);
		let client = PolicyClient::new(setup_proxy_test("{}").unwrap().pi)
			.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Oidc)
			.with_parent(&parent);
		let send = || {
			let req = http::Request::builder()
				.uri(server.uri())
				.header("authorization", "Bearer outbound")
				.body(Body::empty())
				.unwrap();
			client.simple_call(req)
		};
		let (a, b) = tokio::join!(send(), send());
		assert_eq!(a.unwrap().status(), 401);
		assert_eq!(b.unwrap().status(), 401);
		let calls = calls.take();
		assert_eq!(calls.len(), 2);
		assert_eq!(parent.headers()["authorization"], "Bearer inbound");
		let exec = Executor {
			upstream_calls: Some(&calls),
			..Default::default()
		};
		assert!(
			exec.eval_bool(
				&Expression::new_strict(
					r#"
			upstreamCalls.all(c, c.kind == "Policy" && c.subtype == "Oidc" &&
				c.request.headers.authorization == "Bearer outbound" &&
				c.response.code == 401 && c.duration >= duration("0s") && c.error == null)
		"#
				)
				.unwrap()
			)
		);
	}

	#[test]
	fn upstream_calls_preserve_failures_and_empty_lists() {
		let calls = UpstreamCalls::default();
		let labels = OutboundCallLabels {
			kind: OutboundCallKind::Primary,
			subtype: OutboundCallSubtype::Mcp,
		};
		{
			let mut call = PendingCall::new(
				calls.clone(),
				labels,
				"search".to_owned(),
				&http::HeaderMap::new(),
			);
			call.set_error("connection refused".to_owned());
		}
		let records = calls.take();
		let exec = Executor {
			upstream_calls: Some(&records),
			..Default::default()
		};
		assert!(exec.eval_bool(&Expression::new_strict(r#"upstreamCalls[0].target == "search" && upstreamCalls[0].error == "connection refused" && upstreamCalls[0].response.code == null"#).unwrap()));
		let empty = calls.take();
		let exec = Executor {
			upstream_calls: Some(&empty),
			..Default::default()
		};
		assert!(exec.eval_bool(&Expression::new_strict("size(upstreamCalls) == 0").unwrap()));
	}
}
