use std::collections::HashMap;
use std::sync::Arc;

use prost_wkt_types::Struct;
use rmcp::model::{ErrorCode, ErrorData};
use serde_json::Value;
use tracing::{debug, warn};

use crate::cel;
use crate::http::envoy_proto_common::{json_to_prost_value, json_to_struct};
use crate::http::ext_proc::GrpcReferenceChannel;
use crate::mcp::extmcp::wire::ext_mcp_client::ExtMcpClient;
use crate::mcp::extmcp::wire::{
	self, AuthorizationError, McpRequest, McpResponse, mcp_request_result, mcp_response_result,
};
use crate::mcp::extmcp::{FailureMode, Outcome, Remote};
use crate::mcp::upstream::IncomingRequestContext;
use crate::proxy::httpproxy::PolicyClient;

pub(crate) enum RequestOutcome {
	Pass,
	Mutated(Value),
	Reject(ErrorData),
}

pub(crate) async fn check_request(
	remote: &Remote,
	method: &str,
	backend: &str,
	body: Option<Value>,
	req_ctx: &IncomingRequestContext,
	client: &PolicyClient,
) -> RequestOutcome {
	let mcp_request = serialize_body(body);
	let metadata_context = build_metadata(&remote.metadata, req_ctx);
	let req = McpRequest {
		service_name: backend.to_string(),
		method: method.to_string(),
		metadata_context,
		mcp_request,
	};
	let mut grpc = build_client(remote, client.clone());
	let tonic_req = tonic::Request::new(req);
	let result = match grpc.check_request(tonic_req).await {
		Ok(resp) => resp.into_inner().result,
		Err(status) => {
			return on_grpc_error(remote, method, backend, "checkRequest", status);
		},
	};
	match result {
		Some(mcp_request_result::Result::Pass(_)) => RequestOutcome::Pass,
		Some(mcp_request_result::Result::Mutated(s)) => match struct_to_json(&s) {
			Ok(v) => RequestOutcome::Mutated(v),
			Err(e) => on_protocol_violation(remote, method, backend, &format!("mutated decode: {e}")),
		},
		Some(mcp_request_result::Result::Error(e)) => RequestOutcome::Reject(translate_error(e)),
		None => on_protocol_violation(remote, method, backend, "missing result oneof"),
	}
}

pub(crate) async fn check_response(
	remote: &Remote,
	method: &str,
	backend: &str,
	body: &mut Value,
	req_ctx: &IncomingRequestContext,
	client: &PolicyClient,
) -> Outcome {
	let mcp_response = serialize_body(Some(body.clone()));
	let metadata_context = build_metadata(&remote.metadata, req_ctx);
	let req = McpResponse {
		service_name: backend.to_string(),
		method: method.to_string(),
		metadata_context,
		mcp_response,
	};
	let mut grpc = build_client(remote, client.clone());
	let tonic_req = tonic::Request::new(req);
	let result = match grpc.check_response(tonic_req).await {
		Ok(resp) => resp.into_inner().result,
		Err(status) => {
			return match on_grpc_error(remote, method, backend, "checkResponse", status) {
				RequestOutcome::Pass => Outcome::Pass,
				RequestOutcome::Reject(e) => Outcome::Reject(e),
				// Contract violations on response phase coerce to failure_mode again.
				_ => fail_outcome(remote),
			};
		},
	};
	match result {
		Some(mcp_response_result::Result::Pass(_)) => Outcome::Pass,
		Some(mcp_response_result::Result::Mutated(s)) => match struct_to_json(&s) {
			Ok(v) => {
				*body = v;
				Outcome::Mutated
			},
			Err(e) => {
				warn!(method, backend, error = %e, "extMcp: response mutated decode failed");
				fail_outcome(remote)
			},
		},
		Some(mcp_response_result::Result::Error(e)) => Outcome::Reject(translate_error(e)),
		None => {
			warn!(method, backend, "extMcp: response missing result oneof");
			fail_outcome(remote)
		},
	}
}

fn build_metadata(
	cfg: &HashMap<String, Arc<cel::Expression>>,
	req_ctx: &IncomingRequestContext,
) -> Option<Struct> {
	if cfg.is_empty() {
		return None;
	}
	let cel_req = req_ctx.as_request();
	let exec = cel::Executor::new_request(&cel_req);
	let fields = cfg
		.iter()
		.filter_map(|(k, expr)| match eval_to_value(&exec, expr) {
			Ok(v) => Some((k.clone(), v)),
			Err(e) => {
				warn!(key = %k, error = %e, "extMcp: metadata CEL expression failed; skipping");
				None
			},
		})
		.collect();
	Some(Struct { fields })
}

fn eval_to_value(
	exec: &cel::Executor<'_>,
	expr: &cel::Expression,
) -> anyhow::Result<prost_wkt_types::Value> {
	let v = exec.eval(expr)?;
	let js = v.json().map_err(|_| cel::Error::JsonConvert)?;
	Ok(json_to_prost_value(js)?)
}

fn build_client(remote: &Remote, client: PolicyClient) -> ExtMcpClient<GrpcReferenceChannel> {
	ExtMcpClient::new(GrpcReferenceChannel {
		target: remote.target.clone(),
		client,
		policies: Arc::new(Vec::new()),
	})
}

fn serialize_body(body: Option<Value>) -> Option<Struct> {
	let v = body?;
	match json_to_struct(v) {
		Ok(s) => Some(s),
		Err(e) => {
			warn!(error = %e, "extMcp: failed to encode body as Struct; sending empty");
			None
		},
	}
}

fn struct_to_json(s: &Struct) -> Result<Value, serde_json::Error> {
	// prost_wkt_types::Struct serializes as a JSON object directly.
	serde_json::to_value(s)
}

fn translate_error(e: AuthorizationError) -> ErrorData {
	use wire::authorization_error::Code as C;
	let code = match C::try_from(e.code).unwrap_or(C::Unknown) {
		C::PermissionDenied => ErrorCode(-32001),
		C::ResourceExhausted => ErrorCode(-32002),
		C::Invalid => ErrorCode(-32600),
		C::Unknown => ErrorCode(-32603),
	};
	let data = e
		.mcp_error
		.as_ref()
		.and_then(|s| serde_json::to_value(s).ok());
	ErrorData::new(code, e.reason, data)
}

fn on_grpc_error(
	remote: &Remote,
	method: &str,
	backend: &str,
	rpc: &str,
	status: tonic::Status,
) -> RequestOutcome {
	debug!(method, backend, rpc, code = ?status.code(), message = %status.message(), "extMcp: gRPC error");
	match remote.failure_mode {
		FailureMode::Allow => RequestOutcome::Pass,
		FailureMode::Deny => RequestOutcome::Reject(ErrorData::new(
			ErrorCode(-32603),
			format!("extMcp {rpc} failed: {}", status.message()),
			None,
		)),
	}
}

fn on_protocol_violation(
	remote: &Remote,
	method: &str,
	backend: &str,
	reason: &str,
) -> RequestOutcome {
	warn!(method, backend, reason, "extMcp: protocol violation");
	match remote.failure_mode {
		FailureMode::Allow => RequestOutcome::Pass,
		FailureMode::Deny => RequestOutcome::Reject(ErrorData::new(
			ErrorCode(-32603),
			format!("extMcp protocol violation: {reason}"),
			None,
		)),
	}
}

fn fail_outcome(remote: &Remote) -> Outcome {
	match remote.failure_mode {
		FailureMode::Allow => Outcome::Pass,
		FailureMode::Deny => Outcome::Reject(ErrorData::new(
			ErrorCode(-32603),
			"extMcp internal error".to_string(),
			None,
		)),
	}
}
