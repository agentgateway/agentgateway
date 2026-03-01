use opentelemetry::KeyValue;
use opentelemetry::trace::Status as SpanStatus;

use crate::http::StatusCode;
use crate::llm::LLMRequest;
use crate::telemetry::log;
use crate::telemetry::log::{SpanWriter, StartedSpan};

pub const ATTR_PROTOCOL: &str = "protocol";
pub const ATTR_GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
pub const ATTR_GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
pub const ATTR_GEN_AI_PROVIDER_NAME: &str = "gen_ai.provider.name";
pub const ATTR_GEN_AI_AGENT_NAME: &str = "gen_ai.agent.name";
pub const ATTR_A2A_METHOD: &str = "a2a.method";
pub const ATTR_MCP_METHOD_NAME: &str = "mcp.method.name";
pub const ATTR_MCP_SESSION_ID: &str = "mcp.session.id";
pub const ATTR_JSONRPC_REQUEST_ID: &str = "jsonrpc.request.id";
pub const ATTR_JSONRPC_PROTOCOL_VERSION: &str = "jsonrpc.protocol.version";
pub const ATTR_RPC_RESPONSE_STATUS_CODE: &str = "rpc.response.status_code";
pub const ATTR_MCP_RESOURCE_URI: &str = "mcp.resource.uri";
pub const ATTR_NETWORK_PROTOCOL_NAME: &str = "network.protocol.name";
pub const ATTR_NETWORK_PROTOCOL_VERSION: &str = "network.protocol.version";
pub const ATTR_NETWORK_TRANSPORT: &str = "network.transport";
pub const ATTR_GEN_AI_CONVERSATION_ID: &str = "gen_ai.conversation.id";
pub const ATTR_GATEWAY_TURN_ID: &str = "gateway.turn.id";
pub const ATTR_GATEWAY_SPAN_SCHEMA_VERSION: &str = "gateway.span_schema_version";
pub const ATTR_GATEWAY_MCP_LIFECYCLE_FANOUT: &str = "gateway.mcp.lifecycle.fanout";
pub const ATTR_GATEWAY_MCP_LIFECYCLE_RETRY_COUNT: &str = "gateway.mcp.lifecycle.retry.count";

#[derive(Debug, Clone)]
pub struct OperationSpanFields {
	pub name: String,
	pub attrs: Vec<KeyValue>,
}

#[derive(Debug, Clone, Copy)]
pub enum OperationKind<'a> {
	Llm {
		operation: &'static str,
		model: &'a str,
		provider: &'a str,
	},
	A2a {
		method: &'a str,
		agent_name: Option<&'a str>,
	},
	Mcp {
		method: &'a str,
		target: Option<&'a str>,
		session_id: &'a str,
		request_id: &'a str,
		jsonrpc_protocol_version: Option<&'a str>,
		rpc_response_status_code: Option<i64>,
		resource_uri: Option<&'a str>,
		network_protocol_name: Option<&'a str>,
		network_protocol_version: Option<&'a str>,
		network_transport: Option<&'a str>,
		turn_id: &'a str,
		gen_ai_operation: Option<&'static str>,
		fanout_mode: bool,
		retry_count: u32,
	},
}

pub fn operation_span_fields(kind: OperationKind<'_>) -> OperationSpanFields {
	match kind {
		OperationKind::Llm {
			operation,
			model,
			provider,
		} => OperationSpanFields {
			name: format!("{operation} {model}"),
			attrs: vec![
				KeyValue::new(ATTR_GATEWAY_SPAN_SCHEMA_VERSION, 2_i64),
				KeyValue::new(ATTR_GEN_AI_OPERATION_NAME, operation),
				KeyValue::new(ATTR_PROTOCOL, "llm"),
				KeyValue::new(ATTR_GEN_AI_REQUEST_MODEL, model.to_string()),
				KeyValue::new(ATTR_GEN_AI_PROVIDER_NAME, provider.to_string()),
			],
		},
		OperationKind::A2a { method, agent_name } => {
			let mut attrs = vec![
				KeyValue::new(ATTR_GATEWAY_SPAN_SCHEMA_VERSION, 2_i64),
				KeyValue::new(ATTR_GEN_AI_OPERATION_NAME, "invoke_agent"),
				KeyValue::new(ATTR_PROTOCOL, "a2a"),
				KeyValue::new(ATTR_A2A_METHOD, method.to_string()),
			];
			if let Some(agent_name) = agent_name {
				attrs.push(KeyValue::new(
					ATTR_GEN_AI_AGENT_NAME,
					agent_name.to_string(),
				));
			}
			let name = match agent_name {
				Some(agent_name) if !agent_name.is_empty() => format!("invoke_agent {agent_name}"),
				_ => "invoke_agent".to_string(),
			};
			OperationSpanFields { name, attrs }
		},
		OperationKind::Mcp {
			method,
			target,
			session_id,
			request_id,
			jsonrpc_protocol_version,
			rpc_response_status_code,
			resource_uri,
			network_protocol_name,
			network_protocol_version,
			network_transport,
			turn_id,
			gen_ai_operation,
			fanout_mode,
			retry_count,
		} => {
			let mut attrs = vec![
				KeyValue::new(ATTR_GATEWAY_SPAN_SCHEMA_VERSION, 2_i64),
				KeyValue::new(ATTR_PROTOCOL, "mcp"),
				KeyValue::new(ATTR_MCP_METHOD_NAME, method.to_string()),
				KeyValue::new(ATTR_MCP_SESSION_ID, session_id.to_string()),
				KeyValue::new(ATTR_JSONRPC_REQUEST_ID, request_id.to_string()),
				KeyValue::new(ATTR_GEN_AI_CONVERSATION_ID, session_id.to_string()),
				KeyValue::new(ATTR_GATEWAY_TURN_ID, turn_id.to_string()),
				KeyValue::new(ATTR_GATEWAY_MCP_LIFECYCLE_FANOUT, fanout_mode),
				KeyValue::new(
					ATTR_GATEWAY_MCP_LIFECYCLE_RETRY_COUNT,
					i64::from(retry_count),
				),
			];
			if let Some(gen_ai_operation) = gen_ai_operation {
				attrs.push(KeyValue::new(ATTR_GEN_AI_OPERATION_NAME, gen_ai_operation));
			}
			if let Some(version) = jsonrpc_protocol_version {
				attrs.push(KeyValue::new(
					ATTR_JSONRPC_PROTOCOL_VERSION,
					version.to_string(),
				));
			}
			if let Some(code) = rpc_response_status_code {
				attrs.push(KeyValue::new(ATTR_RPC_RESPONSE_STATUS_CODE, code));
			}
			if let Some(uri) = resource_uri {
				attrs.push(KeyValue::new(ATTR_MCP_RESOURCE_URI, uri.to_string()));
			}
			if let Some(name) = network_protocol_name {
				attrs.push(KeyValue::new(ATTR_NETWORK_PROTOCOL_NAME, name.to_string()));
			}
			if let Some(version) = network_protocol_version {
				attrs.push(KeyValue::new(
					ATTR_NETWORK_PROTOCOL_VERSION,
					version.to_string(),
				));
			}
			if let Some(transport) = network_transport {
				attrs.push(KeyValue::new(ATTR_NETWORK_TRANSPORT, transport.to_string()));
			}
			let name = match target {
				Some(t) if !t.is_empty() => format!("{method} {t}"),
				_ => method.to_string(),
			};
			OperationSpanFields { name, attrs }
		},
	}
}

pub fn request_root_span_name(path_match: Option<&str>) -> String {
	match path_match {
		Some(path_match) => format!("http.request {path_match}"),
		None => "http.request".to_string(),
	}
}

struct OperationSpanContext {
	span: StartedSpan,
}

#[must_use]
pub(crate) struct OperationSpanGuard {
	ctx: Option<OperationSpanContext>,
	status: Option<StatusCode>,
	error_type: Option<&'static str>,
}

impl OperationSpanGuard {
	pub(crate) fn child_writer(&self) -> SpanWriter {
		self
			.ctx
			.as_ref()
			.expect("operation span guard missing context")
			.span
			.child_writer()
	}

	pub(crate) fn mark_status(&mut self, status: StatusCode) {
		self.status = Some(status);
	}

	pub(crate) fn mark_error(&mut self, error_type: &'static str) {
		self.error_type = Some(error_type);
	}
}

impl Drop for OperationSpanGuard {
	fn drop(&mut self) {
		let Some(ctx) = self.ctx.take() else {
			return;
		};
		emit_operation_span(ctx, self.status, self.error_type);
	}
}

pub(crate) fn start_a2a_operation_span(
	root_writer: &SpanWriter,
	method: &str,
	agent_name: Option<&str>,
) -> OperationSpanGuard {
	let span = operation_span_fields(OperationKind::A2a { method, agent_name });
	let span = root_writer.start(span.name, |sb| sb.with_attributes(span.attrs));
	OperationSpanGuard {
		ctx: Some(OperationSpanContext { span }),
		status: None,
		error_type: None,
	}
}

pub(crate) fn start_llm_operation_span(
	root_writer: &SpanWriter,
	llm_request: &LLMRequest,
) -> OperationSpanGuard {
	let provider = log::normalize_gen_ai_provider(llm_request.provider.as_str());
	let span = operation_span_fields(OperationKind::Llm {
		operation: log::gen_ai_operation_name(llm_request.input_format),
		model: llm_request.request_model.as_str(),
		provider: provider.as_ref(),
	});
	let span = root_writer.start(span.name, |sb| sb.with_attributes(span.attrs));
	OperationSpanGuard {
		ctx: Some(OperationSpanContext { span }),
		status: None,
		error_type: None,
	}
}

fn operation_status(status: Option<StatusCode>, error_type: Option<&str>) -> SpanStatus {
	if let Some(error_type) = error_type {
		return SpanStatus::error(error_type.to_string());
	}
	if let Some(status) = status
		&& (status.is_client_error() || status.is_server_error())
	{
		return SpanStatus::error(format!("http {}", status.as_u16()));
	}
	SpanStatus::Unset
}

fn operation_error_type_from_status(status: StatusCode) -> Option<&'static str> {
	if status.is_server_error() {
		Some("http_server_error")
	} else if status.is_client_error() {
		Some("http_client_error")
	} else {
		None
	}
}

fn emit_operation_span(
	mut ctx: OperationSpanContext,
	status: Option<StatusCode>,
	error_type: Option<&'static str>,
) {
	let effective_error_type =
		error_type.or_else(|| status.and_then(operation_error_type_from_status));
	if let Some(error_type) = effective_error_type {
		ctx
			.span
			.set_attributes([KeyValue::new("error.type", error_type)]);
	}
	ctx
		.span
		.set_status(operation_status(status, effective_error_type));
	ctx.span.end();
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn request_root_span_name_is_http_request_scoped() {
		assert_eq!(request_root_span_name(None), "http.request");
		assert_eq!(
			request_root_span_name(Some("/v1/chat/completions")),
			"http.request /v1/chat/completions"
		);
	}

	#[test]
	fn llm_operation_fields_are_typed() {
		let fields = operation_span_fields(OperationKind::Llm {
			operation: "chat",
			model: "gpt-4.1",
			provider: "openai",
		});
		assert_eq!(fields.name, "chat gpt-4.1");
		assert!(
			fields
				.attrs
				.iter()
				.any(|kv| kv.key.as_str() == ATTR_PROTOCOL)
		);
		assert!(
			fields
				.attrs
				.iter()
				.any(|kv| kv.key.as_str() == ATTR_GEN_AI_OPERATION_NAME)
		);
	}
}
