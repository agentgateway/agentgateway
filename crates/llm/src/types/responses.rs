use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::typed::{
	EasyInputContent, EasyInputMessage, InputContent, InputItem, InputMessage, InputRole,
	InputTextContent, OutputItem, OutputMessageContent as Content, OutputTextContent as OutputText,
	Role,
};
use super::*;
use crate::{
	AIError, InputFormat, LLMRequest, LLMRequestParams, LLMResponse, RequestType, ResponseType,
};

/// Raw Responses API input — preserves the wire format for passthrough fidelity.
/// Typed deserialization would reject unknown item shapes (e.g. assistant history).
#[derive(Debug, Deserialize, Clone, Serialize)]
#[serde(untagged)]
pub enum RequestInput {
	Text(String),
	Items(Vec<RawInputItem>),
}

#[derive(Debug, Deserialize, Clone, Serialize, PartialEq)]
#[serde(transparent)]
pub struct RawInputItem(Value);

impl RawInputItem {
	fn from_typed(item: InputItem) -> Self {
		Self(serde_json::to_value(item).expect("responses input item should serialize"))
	}

	pub(crate) fn from_value(item: Value) -> Self {
		Self(item)
	}

	fn from_user_text(text: String) -> Self {
		Self::from_typed(InputItem::from(InputMessage {
			content: vec![InputContent::InputText(InputTextContent {
				text,
				prompt_cache_breakpoint: None,
			})],
			role: InputRole::User,
			status: None,
		}))
	}

	fn from_simple_message(msg: SimpleChatCompletionMessage) -> Self {
		Self::from_typed(InputItem::from(msg))
	}

	fn as_simple_message(&self) -> Option<SimpleChatCompletionMessage> {
		let role = self.0.get("role")?.as_str()?;
		let role = match role {
			"user" => strng::literal!("user"),
			"assistant" => strng::literal!("assistant"),
			"system" => strng::literal!("system"),
			"developer" => strng::literal!("developer"),
			_ => return None,
		};

		let content = match self.0.get("content")? {
			Value::String(text) => strng::new(text),
			Value::Array(parts) => {
				let text = parts
					.iter()
					.filter_map(|part| {
						let part_type = part.get("type")?.as_str()?;
						match part_type {
							"input_text" | "output_text" => part.get("text")?.as_str(),
							_ => None,
						}
					})
					.collect::<Vec<_>>()
					.join("\n");
				strng::new(&text)
			},
			_ => return None,
		};

		Some(SimpleChatCompletionMessage { role, content })
	}

	fn visit_text_mut(&mut self, f: &mut dyn FnMut(ContentScope, &mut String)) {
		if let Some(role) = self.0.get("role").and_then(|r| r.as_str()) {
			let scope = match role {
				"system" | "developer" => ContentScope::SystemPrompt,
				_ => ContentScope::Messages,
			};
			match self.0.get_mut("content") {
				Some(Value::String(text)) => f(scope, text),
				Some(Value::Array(parts)) => {
					// assistant refusal parts carry prose under `refusal`, not `text`
					for part in parts.iter_mut() {
						visit_json_at(part, &["refusal"], scope, f);
					}
					scan_value_text_runs(scope, parts, f);
				},
				_ => {},
			}
			return;
		}
		visit_tool_item_text(&mut self.0, f);
	}
}

// Keep this in sync with the typed output visitor below.
fn visit_tool_item_text(value: &mut Value, f: &mut dyn FnMut(ContentScope, &mut String)) {
	use ContentScope::{ToolInput, ToolOutput};
	if has_signature(value) {
		return;
	}
	match value.get("type").and_then(Value::as_str) {
		Some("function_call_output" | "custom_tool_call_output") => {
			if let Some(output) = value.get_mut("output") {
				visit_tool_output_text(output, f);
			}
		},
		Some("local_shell_call_output" | "apply_patch_call_output") => {
			visit_json_at(value, &["output"], ToolOutput, f)
		},
		Some("shell_call_output") => {
			if let Some(Value::Array(outputs)) = value.get_mut("output") {
				for output in outputs {
					visit_json_at(output, &["stdout"], ToolOutput, f);
					visit_json_at(output, &["stderr"], ToolOutput, f);
				}
			}
		},
		Some("program_output") => visit_json_at(value, &["result"], ToolOutput, f),
		Some("function_call" | "mcp_approval_request") => {
			visit_json_at(value, &["arguments"], ToolInput, f)
		},
		Some("mcp_call") => {
			visit_json_at(value, &["arguments"], ToolInput, f);
			visit_json_at(value, &["output"], ToolOutput, f);
			if let Some(error) = value.get_mut("error") {
				match error {
					Value::String(text) => f(ToolOutput, text),
					_ => {
						visit_json_at(error, &["message"], ToolOutput, f);
						if let Some(content) = error.get_mut("content") {
							visit_tool_output_text(content, f);
						}
					},
				}
			}
		},
		Some("custom_tool_call") => visit_json_at(value, &["input"], ToolInput, f),
		Some("local_shell_call") => {
			for field in ["command", "env", "user", "working_directory"] {
				visit_json_at(value, &["action", field], ToolInput, f);
			}
		},
		Some("shell_call") => visit_json_at(value, &["action", "commands"], ToolInput, f),
		Some("computer_call") => {
			if let Some(action) = value.get_mut("action") {
				visit_computer_action_value(action, f);
			}
			if let Some(Value::Array(actions)) = value.get_mut("actions") {
				for action in actions {
					visit_computer_action_value(action, f);
				}
			}
			visit_safety_check_text(value, "pending_safety_checks", ToolInput, f);
		},
		Some("web_search_call") => {
			for field in ["query", "url", "pattern"] {
				visit_json_at(value, &["action", field], ToolInput, f);
			}
			if let Some(Value::Array(sources)) =
				value.get_mut("action").and_then(|a| a.get_mut("sources"))
			{
				for source in sources {
					visit_json_at(source, &["url"], ToolOutput, f);
				}
			}
		},
		Some("apply_patch_call") => {
			visit_json_at(value, &["operation", "path"], ToolInput, f);
			visit_json_at(value, &["operation", "diff"], ToolInput, f);
		},
		Some("computer_call_output") => {
			visit_safety_check_text(value, "acknowledged_safety_checks", ToolOutput, f)
		},
		Some("file_search_call") => {
			visit_json_at(value, &["queries"], ToolInput, f);
			if let Some(Value::Array(results)) = value.get_mut("results") {
				for result in results {
					for field in ["text", "filename", "attributes"] {
						visit_json_at(result, &[field], ToolOutput, f);
					}
				}
			}
		},
		Some("code_interpreter_call") => {
			visit_json_at(value, &["code"], ToolInput, f);
			if let Some(Value::Array(outputs)) = value.get_mut("outputs") {
				for output in outputs {
					if output.get("type").and_then(Value::as_str) == Some("logs") {
						visit_json_at(output, &["logs"], ToolOutput, f);
					}
				}
			}
		},
		Some("tool_search_call") => visit_json_at(value, &["arguments"], ToolInput, f),
		Some("mcp_list_tools" | "tool_search_output") => {
			if let Some(Value::Array(tools)) = value.get_mut("tools") {
				for tool in tools {
					visit_tool_definition_value(tool, f);
				}
			}
			visit_json_at(value, &["error"], ToolOutput, f);
		},
		Some("mcp_approval_response") => visit_json_at(value, &["reason"], ToolInput, f),
		Some("reasoning") => {
			if value.get("encrypted_content").is_none_or(Value::is_null) {
				for field in ["summary", "content"] {
					if let Some(Value::Array(parts)) = value.get_mut(field) {
						for part in parts {
							visit_json_at(part, &["text"], ContentScope::Messages, f);
						}
					}
				}
			}
		},
		// Tool configuration, opaque media, and signed content are not scanned.
		Some(
			"additional_tools"
			| "item_reference"
			| "compaction_trigger"
			| "image_generation_call"
			| "compaction"
			| "program",
		) => {},
		other => tracing::debug!(
			item_type = other.unwrap_or("<none>"),
			"unrecognized input item; not scanned by prompt guards"
		),
	}
}

fn visit_tool_output_text(value: &mut Value, f: &mut dyn FnMut(ContentScope, &mut String)) {
	match value {
		Value::String(text) => f(ContentScope::ToolOutput, text),
		Value::Array(parts) => {
			for part in parts {
				visit_tool_output_text(part, f);
			}
		},
		Value::Object(_) => match value.get("type").and_then(Value::as_str) {
			Some("input_text" | "text") => visit_json_at(value, &["text"], ContentScope::ToolOutput, f),
			Some("resource") => visit_json_at(value, &["resource", "text"], ContentScope::ToolOutput, f),
			Some("resource_link") => {
				for field in ["title", "description"] {
					visit_json_at(value, &[field], ContentScope::ToolOutput, f);
				}
			},
			_ => {},
		},
		_ => {},
	}
}

fn visit_safety_check_text(
	value: &mut Value,
	field: &str,
	scope: ContentScope,
	f: &mut dyn FnMut(ContentScope, &mut String),
) {
	if let Some(Value::Array(checks)) = value.get_mut(field) {
		for check in checks {
			visit_json_at(check, &["message"], scope, f);
		}
	}
}

fn visit_computer_action_value(value: &mut Value, f: &mut dyn FnMut(ContentScope, &mut String)) {
	match value.get("type").and_then(Value::as_str) {
		Some("type") => visit_json_at(value, &["text"], ContentScope::ToolInput, f),
		Some("keypress") => visit_json_at(value, &["keys"], ContentScope::ToolInput, f),
		_ => {},
	}
}

fn visit_tool_definition_value(value: &mut Value, f: &mut dyn FnMut(ContentScope, &mut String)) {
	for field in ["description", "server_description"] {
		visit_json_at(value, &[field], ContentScope::ToolOutput, f);
	}
	visit_json_at(
		value,
		&["annotations", "title"],
		ContentScope::ToolOutput,
		f,
	);
	for field in ["parameters", "input_schema", "output_schema"] {
		if let Some(schema) = value.get_mut(field) {
			visit_json_schema_text(schema, &mut |text| f(ContentScope::ToolOutput, text));
		}
	}
	if let Some(Value::Array(tools)) = value.get_mut("tools") {
		for tool in tools {
			visit_tool_definition_value(tool, f);
		}
	}
}

fn visit_output_tool_item(item: &mut OutputItem, f: &mut dyn FnMut(ContentScope, &mut String)) {
	use ContentScope::{ToolInput, ToolOutput};
	use async_openai::types::responses as sdk;
	match item {
		OutputItem::FunctionCall(call) => f(ToolInput, &mut call.arguments),
		OutputItem::CustomToolCall(call) => f(ToolInput, &mut call.input),
		OutputItem::FunctionCallOutput(call) => match &mut call.output {
			sdk::FunctionCallOutput::Text(text) => f(ToolOutput, text),
			sdk::FunctionCallOutput::Content(parts) => visit_output_content(parts, f),
		},
		OutputItem::CustomToolCallOutput(call) => match &mut call.output {
			sdk::CustomToolCallOutputOutput::Text(text) => f(ToolOutput, text),
			sdk::CustomToolCallOutputOutput::List(parts) => visit_output_content(parts, f),
		},
		OutputItem::FileSearchCall(call) => {
			for query in &mut call.queries {
				f(ToolInput, query);
			}
			for result in call.results.iter_mut().flatten() {
				f(ToolOutput, &mut result.text);
				f(ToolOutput, &mut result.filename);
				for value in result.attributes.values_mut() {
					visit_json_strings(value, &mut |text| f(ToolOutput, text));
				}
			}
		},
		OutputItem::WebSearchCall(call) => match &mut call.action {
			Some(sdk::WebSearchToolCallAction::Search(action)) => {
				// `query` is deprecated in favor of `queries`, but providers may still send it
				#[allow(deprecated)]
				if let Some(query) = &mut action.query {
					f(ToolInput, query);
				}
				for query in action.queries.iter_mut().flatten() {
					f(ToolInput, query);
				}
				for source in action.sources.iter_mut().flatten() {
					f(ToolOutput, &mut source.url);
				}
			},
			Some(sdk::WebSearchToolCallAction::OpenPage(action)) => {
				if let Some(url) = &mut action.url {
					f(ToolInput, url);
				}
			},
			Some(
				sdk::WebSearchToolCallAction::Find(action)
				| sdk::WebSearchToolCallAction::FindInPage(action),
			) => {
				f(ToolInput, &mut action.url);
				f(ToolInput, &mut action.pattern);
			},
			None => {},
		},
		OutputItem::ComputerCall(call) => {
			for action in call
				.action
				.iter_mut()
				.chain(call.actions.iter_mut().flatten())
			{
				visit_computer_action_text(action, &mut |text| f(ToolInput, text));
			}
			for check in &mut call.pending_safety_checks {
				if let Some(message) = &mut check.message {
					f(ToolInput, message);
				}
			}
		},
		OutputItem::ComputerCallOutput(call) => {
			for check in call.acknowledged_safety_checks.iter_mut().flatten() {
				if let Some(message) = &mut check.message {
					f(ToolOutput, message);
				}
			}
		},
		OutputItem::CodeInterpreterCall(call) => {
			if let Some(code) = &mut call.code {
				f(ToolInput, code);
			}
			for output in call.outputs.iter_mut().flatten() {
				if let sdk::CodeInterpreterToolCallOutput::Logs(logs) = output {
					f(ToolOutput, &mut logs.logs);
				}
			}
		},
		OutputItem::LocalShellCall(call) => {
			for text in call
				.action
				.command
				.iter_mut()
				.chain(call.action.env.values_mut())
				.chain(call.action.user.iter_mut())
				.chain(call.action.working_directory.iter_mut())
			{
				f(ToolInput, text);
			}
		},
		OutputItem::ShellCall(call) => {
			for command in &mut call.action.commands {
				f(ToolInput, command);
			}
		},
		OutputItem::ShellCallOutput(call) => {
			for output in &mut call.output {
				f(ToolOutput, &mut output.stdout);
				f(ToolOutput, &mut output.stderr);
			}
		},
		OutputItem::ApplyPatchCall(call) => match &mut call.operation {
			sdk::ApplyPatchOperation::CreateFile(op) => {
				f(ToolInput, &mut op.path);
				f(ToolInput, &mut op.diff);
			},
			sdk::ApplyPatchOperation::UpdateFile(op) => {
				f(ToolInput, &mut op.path);
				f(ToolInput, &mut op.diff);
			},
			sdk::ApplyPatchOperation::DeleteFile(op) => f(ToolInput, &mut op.path),
		},
		OutputItem::ApplyPatchCallOutput(call) => {
			if let Some(output) = &mut call.output {
				f(ToolOutput, output);
			}
		},
		OutputItem::McpCall(call) => {
			f(ToolInput, &mut call.arguments);
			if let Some(output) = &mut call.output {
				f(ToolOutput, output);
			}
			match &mut call.error {
				Some(sdk::MCPToolCallError::McpProtocolError(error)) => f(ToolOutput, &mut error.message),
				Some(sdk::MCPToolCallError::HttpError(error)) => f(ToolOutput, &mut error.message),
				Some(sdk::MCPToolCallError::McpToolExecutionError(error)) => {
					visit_tool_output_text(&mut error.content, f)
				},
				None => {},
			}
		},
		OutputItem::McpApprovalRequest(call) => f(ToolInput, &mut call.arguments),
		OutputItem::McpListTools(list) => {
			for tool in &mut list.tools {
				if let Some(description) = &mut tool.description {
					f(ToolOutput, description);
				}
				visit_json_schema_text(&mut tool.input_schema, &mut |text| f(ToolOutput, text));
				if let Some(annotations) = &mut tool.annotations {
					visit_json_at(annotations, &["title"], ToolOutput, f);
				}
			}
			if let Some(error) = &mut list.error {
				f(ToolOutput, error);
			}
		},
		OutputItem::ToolSearchCall(call) => {
			visit_json_strings(&mut call.arguments, &mut |text| f(ToolInput, text))
		},
		OutputItem::ToolSearchOutput(output) => {
			for tool in &mut output.tools {
				visit_tool_definition_text(tool, &mut |text| f(ToolOutput, text));
			}
		},
		OutputItem::Program(_) => {},
		OutputItem::ProgramOutput(output) => f(ToolOutput, &mut output.result),
		OutputItem::Reasoning(reasoning) => {
			for sdk::SummaryPart::SummaryText(summary) in &mut reasoning.summary {
				f(ContentScope::Messages, &mut summary.text);
			}
			for sdk::ReasoningItemContent::ReasoningText(content) in
				reasoning.content.iter_mut().flatten()
			{
				f(ContentScope::Messages, &mut content.text);
			}
		},
		// Messages are visited separately; opaque results and tool configuration are preserved.
		OutputItem::Message(_)
		| OutputItem::Compaction(_)
		| OutputItem::ImageGenerationCall(_)
		| OutputItem::AdditionalTools(_) => {},
	}
}

fn visit_output_content(parts: &mut [InputContent], f: &mut dyn FnMut(ContentScope, &mut String)) {
	for part in parts {
		if let InputContent::InputText(text) = part {
			f(ContentScope::ToolOutput, &mut text.text);
		}
	}
}

fn visit_computer_action_text(
	action: &mut async_openai::types::responses::ComputerAction,
	f: &mut dyn FnMut(&mut String),
) {
	use async_openai::types::responses::ComputerAction;
	match action {
		ComputerAction::Type(action) => f(&mut action.text),
		ComputerAction::Keypress(action) => action.keys.iter_mut().for_each(f),
		ComputerAction::Click(_)
		| ComputerAction::DoubleClick(_)
		| ComputerAction::Drag(_)
		| ComputerAction::Move(_)
		| ComputerAction::Screenshot
		| ComputerAction::Scroll(_)
		| ComputerAction::Wait => {},
	}
}

fn visit_tool_definition_text(
	tool: &mut async_openai::types::responses::Tool,
	f: &mut dyn FnMut(&mut String),
) {
	use async_openai::types::responses::{NamespaceToolParamTool, Tool};
	match tool {
		Tool::Function(tool) => {
			if let Some(description) = &mut tool.description {
				f(description);
			}
			for schema in tool
				.parameters
				.iter_mut()
				.chain(tool.output_schema.iter_mut())
			{
				visit_json_schema_text(schema, f);
			}
		},
		Tool::Custom(tool) => {
			if let Some(description) = &mut tool.description {
				f(description);
			}
		},
		Tool::Namespace(tool) => {
			f(&mut tool.description);
			for tool in &mut tool.tools {
				match tool {
					NamespaceToolParamTool::Function(tool) => {
						if let Some(description) = &mut tool.description {
							f(description);
						}
						for schema in tool
							.parameters
							.iter_mut()
							.chain(tool.output_schema.iter_mut())
						{
							visit_json_schema_text(schema, f);
						}
					},
					NamespaceToolParamTool::Custom(tool) => {
						if let Some(description) = &mut tool.description {
							f(description);
						}
					},
				}
			}
		},
		Tool::Mcp(tool) => {
			if let Some(description) = &mut tool.server_description {
				f(description);
			}
		},
		Tool::ToolSearch(tool) => {
			if let Some(description) = &mut tool.description {
				f(description);
			}
			if let Some(schema) = &mut tool.parameters {
				visit_json_schema_text(schema, f);
			}
		},
		_ => {},
	}
}

/// `rest` keys preserved when a masked text run collapses; see `scan_text_runs`.
const PRESERVED_REST_KEYS: &[&str] = &[
	// Anthropic-style cache breakpoint, accepted by some OpenAI-compat providers
	"cache_control",
	// OpenAI explicit prompt-cache breakpoint
	"prompt_cache_breakpoint",
];

fn scan_value_text_runs(
	scope: ContentScope,
	parts: &mut Vec<Value>,
	f: &mut dyn FnMut(ContentScope, &mut String),
) {
	crate::types::scan_text_runs(
		parts,
		"\n",
		|part| {
			if !matches!(
				part.get("type").and_then(|t| t.as_str()),
				Some("input_text" | "output_text")
			) {
				return None;
			}
			match part.get_mut("text") {
				Some(Value::String(text)) => Some(text),
				_ => None,
			}
		},
		// parts are pass-through JSON, so `rest` is the whole part
		|part| Some(part),
		PRESERVED_REST_KEYS,
		&mut |text| f(scope, text),
	);
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Request {
	// Required field for prompt enrichment/guards
	pub input: RequestInput,

	// Fields we actually read for routing/telemetry
	#[serde(skip_serializing_if = "Option::is_none")]
	pub model: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub moderation: Option<Value>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub max_output_tokens: Option<u32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub temperature: Option<f32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub top_p: Option<f32>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub stream: Option<bool>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub instructions: Option<String>,

	#[serde(skip_serializing_if = "Option::is_none")]
	pub vendor_extensions: Option<RequestVendorExtensions>,

	// Everything else (tools, reasoning, etc.) - passthrough
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize, Default)]
pub struct RequestVendorExtensions {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub thinking_budget_tokens: Option<u64>,

	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Response {
	pub id: String,
	pub status: String,
	pub output: Vec<OutputItem>,
	pub model: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub service_tier: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub usage: Option<Usage>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Usage {
	pub input_tokens: u64,
	pub output_tokens: u64,
	/// Breakdown of tokens used in a completion.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_tokens_details: Option<UsageInputDetails>,
	/// Breakdown of tokens used in the prompt.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_tokens_details: Option<UsageOutputDetails>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub total_tokens: Option<u64>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct UsageOutputDetails {
	pub reasoning_tokens: Option<u64>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct UsageInputDetails {
	pub cached_tokens: Option<u64>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub cache_write_tokens: Option<u64>,
	#[serde(flatten, default)]
	pub rest: serde_json::Value,
}

pub struct ResponseBuilder {
	response_id: String,
	model: String,
	created_at: u64,
}

impl ResponseBuilder {
	pub fn new(response_id: impl Into<String>, model: impl Into<String>) -> Self {
		Self {
			response_id: response_id.into(),
			model: model.into(),
			created_at: chrono::Utc::now().timestamp() as u64,
		}
	}

	#[allow(deprecated)]
	pub fn response(
		&self,
		status: typed::Status,
		usage: Option<typed::ResponseUsage>,
		error: Option<typed::ResponseError>,
		incomplete_details: Option<typed::IncompleteDetails>,
	) -> typed::Response {
		typed::Response {
			background: None,
			billing: None,
			conversation: None,
			created_at: self.created_at,
			completed_at: None,
			error,
			id: self.response_id.clone(),
			incomplete_details,
			instructions: None,
			max_output_tokens: None,
			metadata: None,
			model: self.model.clone(),
			moderation: None,
			object: "response".to_string(),
			output: Vec::new(),
			parallel_tool_calls: None,
			previous_response_id: None,
			prompt: None,
			prompt_cache_key: None,
			prompt_cache_options: None,
			prompt_cache_retention: None,
			reasoning: None,
			safety_identifier: None,
			service_tier: None,
			status,
			temperature: None,
			text: None,
			tool_choice: None,
			tools: None,
			top_logprobs: None,
			top_p: None,
			truncation: None,
			usage,
			prompt_cache_diagnostics: None,
		}
	}

	pub fn created_event(&self, sequence_number: u64) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseCreated(typed::ResponseCreatedEvent {
			sequence_number,
			response: self.response(typed::Status::InProgress, None, None, None),
		})
	}

	pub fn completed_event(
		&self,
		sequence_number: u64,
		usage: Option<typed::ResponseUsage>,
	) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseCompleted(typed::ResponseCompletedEvent {
			sequence_number,
			response: self.response(typed::Status::Completed, usage, None, None),
		})
	}

	pub fn incomplete_event(
		&self,
		sequence_number: u64,
		usage: Option<typed::ResponseUsage>,
		incomplete_details: typed::IncompleteDetails,
	) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseIncomplete(typed::ResponseIncompleteEvent {
			sequence_number,
			response: self.response(
				typed::Status::Incomplete,
				usage,
				None,
				Some(incomplete_details),
			),
		})
	}

	pub fn failed_event(
		&self,
		sequence_number: u64,
		usage: Option<typed::ResponseUsage>,
		error: typed::ResponseError,
	) -> typed::ResponseStreamEvent {
		typed::ResponseStreamEvent::ResponseFailed(typed::ResponseFailedEvent {
			sequence_number,
			response: self.response(typed::Status::Failed, usage, Some(error), None),
		})
	}
}

impl From<SimpleChatCompletionMessage> for InputItem {
	fn from(msg: SimpleChatCompletionMessage) -> Self {
		match msg.role.as_str() {
			"assistant" => InputItem::EasyMessage(EasyInputMessage {
				r#type: Default::default(),
				role: Role::Assistant,
				content: EasyInputContent::Text(msg.content.to_string()),
				phase: None,
			}),
			"system" => InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: msg.content.to_string(),
					prompt_cache_breakpoint: None,
				})],
				role: InputRole::System,
				status: None,
			}),
			"developer" => InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: msg.content.to_string(),
					prompt_cache_breakpoint: None,
				})],
				role: InputRole::Developer,
				status: None,
			}),
			_ => InputItem::from(InputMessage {
				content: vec![InputContent::InputText(InputTextContent {
					text: msg.content.to_string(),
					prompt_cache_breakpoint: None,
				})],
				role: InputRole::User,
				status: None,
			}),
		}
	}
}

impl Request {
	fn take_input_as_items(&mut self) -> Vec<RawInputItem> {
		match std::mem::replace(&mut self.input, RequestInput::Items(Vec::new())) {
			RequestInput::Text(text) => vec![RawInputItem::from_user_text(text)],
			RequestInput::Items(items) => items,
		}
	}
}

impl RequestType for Request {
	fn input_format() -> crate::InputFormat {
		crate::InputFormat::Responses
	}
	fn body_is_json(&self) -> bool {
		true
	}
	fn model(&mut self) -> &mut Option<String> {
		&mut self.model
	}

	fn to_value(&self) -> serde_json::Result<serde_json::Value> {
		serde_json::to_value(self)
	}

	fn prepend_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		let mut items = self.take_input_as_items();
		let prepend_items: Vec<RawInputItem> = prompts
			.into_iter()
			.map(RawInputItem::from_simple_message)
			.collect();
		items.splice(0..0, prepend_items);
		self.input = RequestInput::Items(items);
	}

	fn append_prompts(&mut self, prompts: Vec<SimpleChatCompletionMessage>) {
		let mut items = self.take_input_as_items();
		items.extend(prompts.into_iter().map(RawInputItem::from_simple_message));
		self.input = RequestInput::Items(items);
	}

	fn to_llm_request(&self, provider: Strng, tokenize: bool) -> Result<LLMRequest, AIError> {
		let model = strng::new(self.model.as_deref().unwrap_or_default());
		let input_tokens = if tokenize {
			let messages = self.get_messages();
			let tokens = crate::tokenizer::num_tokens_from_messages(&model, &messages)?;
			Some(tokens)
		} else {
			None
		};
		Ok(LLMRequest {
			input_tokens,
			input_format: InputFormat::Responses,
			cache_convention: crate::CacheTokenConvention::pending(),
			request_model: model,
			provider,
			streaming: self.stream.unwrap_or_default(),
			params: LLMRequestParams {
				temperature: self.temperature.map(Into::into),
				top_p: self.top_p.map(Into::into),
				frequency_penalty: None,
				presence_penalty: None,
				seed: None,
				max_tokens: self.max_output_tokens.map(Into::into),
				encoding_format: None,
				dimensions: None,
			},
			prompt: Default::default(),
			provider_state: None,
		})
	}

	fn get_messages(&self) -> Vec<SimpleChatCompletionMessage> {
		let mut messages = self
			.instructions
			.as_ref()
			.map(|instructions| SimpleChatCompletionMessage {
				role: strng::literal!("system"),
				content: strng::new(instructions),
			})
			.into_iter()
			.collect::<Vec<_>>();
		messages.extend(match &self.input {
			RequestInput::Text(text) => {
				vec![SimpleChatCompletionMessage {
					role: strng::literal!("user"),
					content: strng::new(text),
				}]
			},
			RequestInput::Items(items) => items
				.iter()
				.filter_map(RawInputItem::as_simple_message)
				.collect(),
		});
		messages
	}

	fn get_messages_v2(&self) -> Vec<NormalizedMessage> {
		let mut messages = self
			.instructions
			.as_ref()
			.map(|instructions| NormalizedMessage {
				role: strng::literal!("system"),
				parts: vec![NormalizedMessagePart::text(strng::new(instructions))],
			})
			.into_iter()
			.collect::<Vec<_>>();
		match &self.input {
			RequestInput::Text(text) => messages.push(NormalizedMessage {
				role: strng::literal!("user"),
				parts: vec![NormalizedMessagePart::text(strng::new(text))],
			}),
			RequestInput::Items(items) => {
				messages.extend(
					items
						.iter()
						.filter_map(|item| normalized_response_item(&item.0)),
				);
			},
		}
		crate::types::attach_tool_result_names(&mut messages);
		messages
	}

	fn set_messages(&mut self, mut messages: Vec<SimpleChatCompletionMessage>) {
		if self.instructions.is_some() {
			self.instructions = messages
				.first()
				.filter(|message| matches!(message.role.as_str(), "developer" | "system"))
				.map(|message| message.content.to_string());
			if self.instructions.is_some() {
				messages.remove(0);
			}
		}
		self.input = RequestInput::Items(
			messages
				.into_iter()
				.map(RawInputItem::from_simple_message)
				.collect(),
		);
	}

	fn visit_text_mut(&mut self, f: &mut dyn FnMut(ContentScope, &mut String)) {
		if let Some(instructions) = &mut self.instructions {
			f(ContentScope::SystemPrompt, instructions);
		}
		match &mut self.input {
			RequestInput::Text(text) => f(ContentScope::Messages, text),
			RequestInput::Items(items) => {
				for item in items {
					item.visit_text_mut(f);
				}
			},
		}
	}
}

fn normalized_response_item(item: &Value) -> Option<NormalizedMessage> {
	if let Some(role) = item.get("role").and_then(Value::as_str) {
		let mut parts = match item.get("content") {
			Some(Value::String(text)) => vec![NormalizedMessagePart::text(strng::new(text))],
			Some(Value::Array(content)) => content
				.iter()
				.filter_map(|part| {
					part
						.get("text")
						.or_else(|| part.get("refusal"))
						.and_then(Value::as_str)
						.map(|text| NormalizedMessagePart::text(strng::new(text)))
				})
				.collect(),
			_ => Vec::new(),
		};
		parts.extend(
			item
				.get("tool_calls")
				.and_then(Value::as_array)
				.into_iter()
				.flatten()
				.filter_map(crate::types::normalized_tool_call),
		);
		return (!parts.is_empty()).then(|| NormalizedMessage {
			role: strng::new(role),
			parts,
		});
	}

	let item_type = item.get("type").and_then(Value::as_str)?;
	if item_type == "reasoning" {
		return Some(NormalizedMessage {
			role: strng::literal!("assistant"),
			parts: vec![NormalizedMessagePart::reasoning(item.clone())],
		});
	}

	let is_call = item_type.ends_with("_call")
		|| matches!(
			item_type,
			"program" | "mcp_approval_request" | "tool_search_call"
		);
	if is_call {
		let name = item
			.get("name")
			.and_then(Value::as_str)
			.or_else(|| item_type.strip_suffix("_call"))
			.unwrap_or(item_type);
		let id = item
			.get("call_id")
			.or_else(|| item.get("id"))
			.and_then(Value::as_str)
			.unwrap_or(name);
		let arguments = [
			"arguments",
			"input",
			"action",
			"actions",
			"operation",
			"queries",
			"code",
		]
		.into_iter()
		.find_map(|key| item.get(key))
		.cloned()
		.unwrap_or_else(|| Value::Object(Default::default()));
		let mut parts = vec![NormalizedMessagePart::tool_call(
			strng::new(id),
			strng::new(name),
			crate::types::parse_json_string(arguments),
		)];
		if let Some(content) = item
			.get("output")
			.or_else(|| item.get("outputs"))
			.or_else(|| item.get("results"))
			.or_else(|| item.get("error"))
		{
			parts.push(NormalizedMessagePart::tool_result(
				Some(strng::new(id)),
				Some(strng::new(name)),
				content.clone(),
				item.get("error").map(|_| true),
			));
		}
		return Some(NormalizedMessage {
			role: strng::literal!("assistant"),
			parts,
		});
	}

	let is_result = item_type.ends_with("_call_output")
		|| matches!(
			item_type,
			"program_output" | "tool_search_output" | "mcp_list_tools"
		);
	if is_result {
		let id = item
			.get("call_id")
			.or_else(|| item.get("id"))
			.and_then(Value::as_str)
			.map(strng::new);
		let content = ["output", "result", "tools", "error"]
			.into_iter()
			.find_map(|key| item.get(key))
			.cloned()
			.unwrap_or(Value::Null);
		return Some(NormalizedMessage {
			role: strng::literal!("tool"),
			parts: vec![NormalizedMessagePart::tool_result(
				id,
				item.get("name").and_then(Value::as_str).map(strng::new),
				content,
				item.get("error").map(|_| true),
			)],
		});
	}
	None
}

fn extract_output_messages(resp: &Response) -> Option<Vec<OutputMessage>> {
	let content: Vec<_> = resp
		.output
		.iter()
		.filter_map(output_item_tool_call_part)
		.collect();

	if content.is_empty() {
		return None;
	}

	Some(vec![OutputMessage {
		role: strng::literal!("assistant"),
		content,
		finish_reason: Some(strng::new(&resp.status)),
	}])
}

pub(crate) fn output_item_tool_call_part(item: &OutputItem) -> Option<OutputMessagePart> {
	let (id, name, arguments) = match item {
		OutputItem::FunctionCall(call) => {
			let arguments = match serde_json::from_str(&call.arguments) {
				Ok(arguments) => arguments,
				Err(_) if call.arguments.trim().is_empty() => serde_json::Value::Object(Default::default()),
				Err(_) => serde_json::Value::String(call.arguments.clone()),
			};
			let name = call
				.namespace
				.as_ref()
				.filter(|namespace| !namespace.is_empty())
				.map_or_else(
					|| call.name.clone(),
					|namespace| {
						format!(
							"{namespace}{}{}",
							crate::conversion::namespace_tools::NAMESPACE_SEPARATOR,
							call.name
						)
					},
				);
			(&call.call_id, name, arguments)
		},
		OutputItem::CustomToolCall(call) => {
			let arguments = match serde_json::from_str(&call.input) {
				Ok(arguments) => arguments,
				Err(_) if call.input.trim().is_empty() => serde_json::Value::Object(Default::default()),
				Err(_) => serde_json::Value::String(call.input.clone()),
			};
			(&call.call_id, call.name.clone(), arguments)
		},
		_ => return None,
	};
	Some(OutputMessagePart::ToolCall {
		id: strng::new(id),
		name: strng::new(&name),
		arguments,
	})
}

impl ResponseType for Response {
	fn to_llm_response(&self, log_content: crate::LogContentFields) -> LLMResponse {
		let output_messages = if log_content.tool_calls {
			extract_output_messages(self)
		} else {
			None
		};

		LLMResponse {
			input_tokens: self.usage.as_ref().map(|u| u.input_tokens),
			input_image_tokens: None,
			input_text_tokens: None,
			input_audio_tokens: None,
			output_tokens: self.usage.as_ref().map(|u| u.output_tokens),
			// Note: responses supports image generation, but it does not report image generation as tokens.
			// Instead there is a cost based on the image parameters (https://platform.openai.com/docs/guides/image-generation#calculating-costs)
			// which we do not currently emit.
			output_image_tokens: None,
			output_text_tokens: None,
			output_audio_tokens: None,
			count_tokens: None,
			total_tokens: self
				.usage
				.as_ref()
				.map(|u| u.total_tokens.unwrap_or(u.input_tokens + u.output_tokens)),
			pages: None,
			reasoning_tokens: self.usage.as_ref().and_then(|u| {
				u.output_tokens_details
					.as_ref()
					.and_then(|d| d.reasoning_tokens)
			}),
			cached_input_tokens: self.usage.as_ref().and_then(|u| {
				u.input_tokens_details
					.as_ref()
					.and_then(|d| d.cached_tokens)
			}),
			cache_creation_input_tokens: self.usage.as_ref().and_then(|u| {
				u.input_tokens_details
					.as_ref()
					.and_then(|d| d.cache_write_tokens)
			}),
			service_tier: self.service_tier.as_deref().map(Into::into),
			provider_model: Some(strng::new(&self.model)),
			completion: if log_content.completion {
				Some(
					self
						.output
						.iter()
						.filter_map(|o| match o {
							OutputItem::Message(msg) => Some(msg),
							_ => None,
						})
						.flat_map(|msg| {
							msg.content.iter().filter_map(|c| match c {
								Content::OutputText(t) => Some(t.text.clone()),
								_ => None,
							})
						})
						.collect(),
				)
			} else {
				None
			},
			output_messages,
			first_token: Default::default(),
			last_token_at: Default::default(),
			inter_chunk_latencies: Default::default(),
		}
	}

	fn to_webhook_choices(&self) -> Vec<crate::webhook::ResponseChoice> {
		self
			.output
			.iter()
			.filter_map(|o| match o {
				OutputItem::Message(msg) => {
					// Extract text from message content
					let content = msg
						.content
						.iter()
						.filter_map(|c| match c {
							Content::OutputText(t) => Some(t.text.clone()),
							_ => None,
						})
						.collect::<Vec<_>>()
						.join("\n");

					Some(crate::webhook::ResponseChoice {
						message: crate::webhook::Message {
							role: "assistant".into(),
							content: content.into(),
						},
					})
				},
				_ => None, // Ignore non-message outputs (tool calls, reasoning, etc.)
			})
			.collect()
	}

	fn set_webhook_choices(
		&mut self,
		choices: Vec<crate::webhook::ResponseChoice>,
	) -> anyhow::Result<()> {
		// Filter only Message outputs (ignore tool calls, reasoning, etc.)
		let message_outputs: Vec<_> = self
			.output
			.iter_mut()
			.filter_map(|o| match o {
				OutputItem::Message(msg) => Some(msg),
				_ => None,
			})
			.collect();

		if message_outputs.len() != choices.len() {
			anyhow::bail!("webhook response message count mismatch");
		}

		for (msg, wh) in message_outputs.into_iter().zip(choices) {
			// Replace message content with webhook's modified content
			msg.content = vec![Content::OutputText(OutputText {
				annotations: vec![],
				logprobs: None,
				text: wh.message.content.to_string(),
			})];
		}
		Ok(())
	}

	fn serialize(&self) -> serde_json::Result<Vec<u8>> {
		serde_json::to_vec(&self)
	}

	fn visit_text_mut(&mut self, f: &mut dyn FnMut(crate::types::ResponseText, &mut String)) {
		for o in &mut self.output {
			if let OutputItem::Program(program) = o {
				f(
					crate::types::ResponseText {
						scope: ContentScope::ToolInput,
						signed: true,
					},
					&mut program.code,
				);
				continue;
			}
			let mut plain = |scope: ContentScope, text: &mut String| f(scope.into(), text);
			let f = &mut plain;
			let OutputItem::Message(msg) = o else {
				visit_output_tool_item(o, f);
				continue;
			};
			for c in &mut msg.content {
				if let Content::Refusal(refusal) = c {
					f(ContentScope::Messages, &mut refusal.refusal);
				}
				if let Content::OutputText(t) = c {
					if t.annotations.is_empty() && t.logprobs.is_none() {
						f(ContentScope::Messages, &mut t.text);
						continue;
					}
					// offset-based metadata cannot survive a text rewrite
					let original = t.text.clone();
					f(ContentScope::Messages, &mut t.text);
					if t.text != original {
						t.annotations.clear();
						t.logprobs = None;
					}
				}
			}
		}
	}
}

pub mod typed {
	use async_openai::types::responses as openai_responses;
	// Re-export async-openai Responses API types for cleaner usage
	pub use async_openai::types::responses::{
		Annotation, AssistantRole, CreateResponse, CustomToolCallOutput, CustomToolCallOutputOutput,
		EasyInputContent, EasyInputMessage, FunctionCallOutput, FunctionToolCall, IncompleteDetails,
		InputContent, InputItem, InputMessage, InputParam, InputRole, InputTextContent,
		InputTokenDetails, Item, MessageItem, OutputContent, OutputItem, OutputMessage,
		OutputMessageContent, OutputStatus, OutputTextContent, OutputTokenDetails,
		PromptCacheBreakpointConfig, Reasoning, ReasoningEffort, ReasoningItem, ReasoningItemContent,
		ReasoningTextContent, Response, ResponseCompletedEvent, ResponseContentPartAddedEvent,
		ResponseContentPartDoneEvent, ResponseCreatedEvent, ResponseError, ResponseErrorCode,
		ResponseErrorEvent, ResponseFailedEvent, ResponseFunctionCallArgumentsDeltaEvent,
		ResponseFunctionCallArgumentsDoneEvent, ResponseInProgressEvent, ResponseIncompleteEvent,
		ResponseOutputItemAddedEvent, ResponseOutputItemDoneEvent, ResponseReasoningTextDeltaEvent,
		ResponseReasoningTextDoneEvent, ResponseRefusalDeltaEvent, ResponseRefusalDoneEvent,
		ResponseTextDeltaEvent, ResponseTextDoneEvent, ResponseTextParam, ResponseUsage, Role, Status,
		TextResponseFormatConfiguration, Tool, ToolChoiceFunction, ToolChoiceOptions, ToolChoiceParam,
	};
	use serde::{Deserialize, Serialize};

	/// Event types for streaming responses from the Responses API (minimal strict subset).
	#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
	#[allow(clippy::enum_variant_names)]
	#[serde(tag = "type")]
	pub enum ResponseStreamEvent {
		/// An event that is emitted when a response is created.
		#[serde(rename = "response.created")]
		ResponseCreated(openai_responses::ResponseCreatedEvent),
		/// Emitted when a response is in progress (intermediate progress event).
		#[serde(rename = "response.in_progress")]
		ResponseInProgress(openai_responses::ResponseInProgressEvent),
		/// Emitted when a new output item is added.
		#[serde(rename = "response.output_item.added")]
		ResponseOutputItemAdded(openai_responses::ResponseOutputItemAddedEvent),
		/// Emitted when a new content part is added.
		#[serde(rename = "response.content_part.added")]
		ResponseContentPartAdded(openai_responses::ResponseContentPartAddedEvent),
		/// Emitted when there is an additional text delta.
		#[serde(rename = "response.output_text.delta")]
		ResponseOutputTextDelta(openai_responses::ResponseTextDeltaEvent),
		/// Emitted when text content is finalized.
		#[serde(rename = "response.output_text.done")]
		ResponseOutputTextDone(openai_responses::ResponseTextDoneEvent),
		/// Emitted when there is an additional reasoning text delta.
		#[serde(rename = "response.reasoning_text.delta")]
		ResponseReasoningTextDelta(openai_responses::ResponseReasoningTextDeltaEvent),
		/// Emitted when reasoning text is finalized.
		#[serde(rename = "response.reasoning_text.done")]
		ResponseReasoningTextDone(openai_responses::ResponseReasoningTextDoneEvent),
		/// Emitted when there is a partial refusal text.
		#[serde(rename = "response.refusal.delta")]
		ResponseRefusalDelta(openai_responses::ResponseRefusalDeltaEvent),
		/// Emitted when refusal text is finalized.
		#[serde(rename = "response.refusal.done")]
		ResponseRefusalDone(openai_responses::ResponseRefusalDoneEvent),
		/// Emitted when there is a partial function-call arguments delta.
		#[serde(rename = "response.function_call_arguments.delta")]
		ResponseFunctionCallArgumentsDelta(openai_responses::ResponseFunctionCallArgumentsDeltaEvent),
		/// Emitted when function-call arguments are finalized.
		#[serde(rename = "response.function_call_arguments.done")]
		ResponseFunctionCallArgumentsDone(openai_responses::ResponseFunctionCallArgumentsDoneEvent),
		/// Emitted when a content part is done.
		#[serde(rename = "response.content_part.done")]
		ResponseContentPartDone(openai_responses::ResponseContentPartDoneEvent),
		/// Emitted when an output item is marked done.
		#[serde(rename = "response.output_item.done")]
		ResponseOutputItemDone(openai_responses::ResponseOutputItemDoneEvent),
		/// Emitted when the model response is complete.
		#[serde(rename = "response.completed")]
		ResponseCompleted(openai_responses::ResponseCompletedEvent),
		/// An event that is emitted when a response finishes as incomplete.
		#[serde(rename = "response.incomplete")]
		ResponseIncomplete(openai_responses::ResponseIncompleteEvent),
		/// An event that is emitted when a response fails.
		#[serde(rename = "response.failed")]
		ResponseFailed(openai_responses::ResponseFailedEvent),
		/// Emitted when an error occurs.
		#[serde(rename = "error")]
		ResponseError(openai_responses::ResponseErrorEvent),
	}
}

#[cfg(test)]
mod tests {
	use super::typed::{FunctionToolCall, OutputStatus};
	use super::*;

	fn response_with_output(output: Vec<OutputItem>) -> Response {
		Response {
			id: "resp_123".to_string(),
			status: "completed".to_string(),
			output,
			model: "gpt-4.1".to_string(),
			service_tier: None,
			usage: None,
			rest: serde_json::Value::Null,
		}
	}

	#[test]
	fn instructions_round_trip_through_messages() {
		let mut request: Request = serde_json::from_value(serde_json::json!({
			"model": "gpt-4.1",
			"instructions": "original instruction",
			"input": [
				{"role": "system", "content": "input system message"},
				{"role": "user", "content": "hello"},
			],
		}))
		.unwrap();
		let mut messages = request.get_messages();
		assert_eq!(messages[0].role.as_str(), "system");
		assert_eq!(messages[1].role.as_str(), "system");
		messages[0].content = strng::literal!("masked instruction");
		messages[1].content = strng::literal!("masked input system message");

		request.set_messages(messages);

		assert_eq!(request.instructions.as_deref(), Some("masked instruction"));
		let input = match &request.input {
			RequestInput::Items(items) => items,
			RequestInput::Text(_) => panic!("rewritten messages should use structured input"),
		};
		assert_eq!(input.len(), 2);
		assert_eq!(input[0].0["role"], "system");
		assert_eq!(input[0].0["content"][0]["type"], "input_text");
		assert_eq!(
			input[0].0["content"][0]["text"],
			"masked input system message"
		);
		assert_eq!(input[1].0["role"], "user");
	}

	#[test]
	fn test_response_tool_calls_populated_when_flag_true() {
		let response = response_with_output(vec![OutputItem::FunctionCall(FunctionToolCall {
			arguments: r#"{"location":"San Francisco"}"#.to_string(),
			call_id: "call_123".to_string(),
			namespace: None,
			name: "get_weather".to_string(),
			caller: None,
			id: Some("fc_123".to_string()),
			status: Some(OutputStatus::Completed),
			r#async: None,
		})]);

		let llm_response = response.to_llm_response(crate::LogContentFields {
			completion: true,
			tool_calls: true,
		});
		let messages = llm_response
			.output_messages
			.expect("output_messages should be present");
		let tool_calls = messages[0].tool_calls();

		assert_eq!(tool_calls.len(), 1);
		assert_eq!(tool_calls[0].id.as_str(), "call_123");
		assert_eq!(tool_calls[0].name.as_str(), "get_weather");
		assert_eq!(
			tool_calls[0].arguments,
			serde_json::json!({"location":"San Francisco"})
		);
	}

	#[test]
	fn test_response_output_messages_omitted_when_flag_false() {
		let response = response_with_output(vec![OutputItem::FunctionCall(FunctionToolCall {
			arguments: "{}".to_string(),
			call_id: "call_123".to_string(),
			namespace: None,
			name: "get_weather".to_string(),
			caller: None,
			id: Some("fc_123".to_string()),
			status: Some(OutputStatus::Completed),
			r#async: None,
		})]);

		let llm_response = response.to_llm_response(crate::LogContentFields::default());

		assert!(llm_response.output_messages.is_none());
	}
}
