use std::collections::HashMap;

use agent_core::strng;
use async_openai::types::responses::{FunctionTool, NamespaceToolParamTool};

use crate::AIError;
use crate::types::responses::typed as responses;

pub(crate) const NAMESPACE_SEPARATOR: &str = "__";

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginalTool {
	namespace: String,
	name: String,
	kind: ToolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
	Function,
	Custom,
}

impl ToolKind {
	/// Returns the Responses API discriminator used for this tool kind.
	/// Mismatch errors use it to describe the client's original protocol shape.
	fn name(self) -> &'static str {
		match self {
			Self::Function => "function",
			Self::Custom => "custom",
		}
	}
}

/// Request-local aliases
#[derive(Debug, Clone, Default)]
pub struct NamespaceToolMap {
	aliases: HashMap<String, OriginalTool>,
}

impl NamespaceToolMap {
	/// Rewrite namespace definitions, forced tool choices, and tool-call history
	/// for Chat Completions and Bedrock Converse. Returns aliases for response restoration.
	/// Bare choices must identify a unique member; qualified `namespace__tool` names
	/// are also accepted. Allowed-tool constraints remain unsupported.
	/// On error the request may be partially rewritten and must be discarded.
	pub fn rewrite_request(req: &mut responses::CreateResponse) -> Result<Self, AIError> {
		let mut map = Self::default();
		map.flatten_tools(&mut req.tools)?;
		let mut names = HashMap::new();
		for tool in req.tools.iter().flatten() {
			let (name, kind) = match tool {
				responses::Tool::Function(tool) => (&tool.name, ToolKind::Function),
				responses::Tool::Custom(tool) => (&tool.name, ToolKind::Custom),
				_ => continue,
			};
			if names.insert(name.as_str(), kind).is_some() {
				return Err(AIError::UnsupportedConversion(strng::format!(
					"duplicate upstream tool name: {name}"
				)));
			}
		}
		map.rewrite_choice(&mut req.tool_choice, &names)?;
		map.rewrite_history(&mut req.input, &names)?;
		Ok(map)
	}

	fn flatten_tools(&mut self, tools: &mut Option<Vec<responses::Tool>>) -> Result<(), AIError> {
		if let Some(tools) = tools {
			for tool in std::mem::take(tools) {
				let responses::Tool::Namespace(namespace) = tool else {
					tools.push(tool);
					continue;
				};
				for member in namespace.tools {
					let (original_name, member_description, kind) = match &member {
						NamespaceToolParamTool::Function(tool) => {
							(&tool.name, &tool.description, ToolKind::Function)
						},
						NamespaceToolParamTool::Custom(tool) => {
							(&tool.name, &tool.description, ToolKind::Custom)
						},
					};
					let name = format!("{}{NAMESPACE_SEPARATOR}{original_name}", namespace.name);

					self.aliases.insert(
						name.clone(),
						OriginalTool {
							namespace: namespace.name.clone(),
							name: original_name.clone(),
							kind,
						},
					);
					// Keep the namespace's instructions visible after removing its container.
					let mut description = member_description.clone();
					if !namespace.description.is_empty() {
						let mut combined = namespace.description.clone();
						if let Some(member) = description.as_ref().filter(|text| !text.is_empty()) {
							combined.push_str("\n\n");
							combined.push_str(member);
						}
						description = Some(combined);
					}
					match member {
						NamespaceToolParamTool::Function(tool) => {
							tools.push(responses::Tool::Function(FunctionTool {
								name,
								description,
								parameters: tool.parameters,
								strict: tool.strict,
								defer_loading: tool.defer_loading,
								allowed_callers: tool.allowed_callers,
								output_schema: tool.output_schema,
								r#async: tool.r#async,
							}));
						},
						NamespaceToolParamTool::Custom(mut tool) => {
							tool.name = name;
							tool.description = description;
							tools.push(responses::Tool::Custom(tool));
						},
					}
				}
			}
		}

		Ok(())
	}

	fn rewrite_choice(
		&self,
		choice: &mut Option<responses::ToolChoiceParam>,
		names: &HashMap<&str, ToolKind>,
	) -> Result<(), AIError> {
		// Neither target conversion can enforce an allowed-tools constraint.
		if matches!(choice, Some(responses::ToolChoiceParam::AllowedTools(_))) {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"allowed_tools tool choice is unsupported for Chat Completions and Bedrock Converse"
			)));
		}

		let (name, kind) = match choice {
			Some(responses::ToolChoiceParam::Function(choice)) => (&mut choice.name, ToolKind::Function),
			Some(responses::ToolChoiceParam::Custom(choice)) => (&mut choice.name, ToolKind::Custom),
			_ => return Ok(()),
		};
		if names.get(name.as_str()) == Some(&kind) {
			return Ok(());
		}

		let original_name = name.clone();
		{
			let mut matches = self
				.aliases
				.iter()
				.filter(|(_, original)| original.name == original_name && original.kind == kind);
			match (matches.next(), matches.next()) {
				(Some((alias, _)), None) => *name = alias.clone(),
				(Some(_), Some(_)) => {
					return Err(AIError::UnsupportedConversion(strng::format!(
						"ambiguous namespaced tool choice: {name}; use namespace__tool to select a member"
					)));
				},
				_ => {
					let wrong_kind = names.get(original_name.as_str()).copied().or_else(|| {
						self
							.aliases
							.values()
							.find(|original| original.name == original_name)
							.map(|original| original.kind)
					});
					if let Some(wrong_kind) = wrong_kind {
						return Err(AIError::UnsupportedConversion(strng::format!(
							"{} tool choice refers to a {} tool: {original_name}",
							kind.name(),
							wrong_kind.name()
						)));
					}
				},
			}
		}

		Ok(())
	}

	fn rewrite_history(
		&mut self,
		input: &mut responses::InputParam,
		names: &HashMap<&str, ToolKind>,
	) -> Result<(), AIError> {
		if let responses::InputParam::Items(items) = input {
			for item in items {
				let (namespace, name, kind) = match item {
					responses::InputItem::Item(responses::Item::FunctionCall(call)) => {
						(&mut call.namespace, &mut call.name, ToolKind::Function)
					},
					responses::InputItem::Item(responses::Item::CustomToolCall(call)) => {
						(&mut call.namespace, &mut call.name, ToolKind::Custom)
					},
					_ => continue,
				};
				let Some(original_namespace) = namespace.as_ref().filter(|ns| !ns.is_empty()) else {
					continue;
				};
				let alias = format!("{original_namespace}{NAMESPACE_SEPARATOR}{name}");
				let original = OriginalTool {
					namespace: original_namespace.clone(),
					name: name.clone(),
					kind,
				};
				let collides = match self.aliases.get(&alias) {
					Some(existing) => {
						existing.namespace != original.namespace || existing.name != original.name
					},
					None => names.contains_key(alias.as_str()),
				};
				if collides {
					return Err(AIError::UnsupportedConversion(strng::format!(
						"history tool call collides with another tool: {alias}"
					)));
				}
				self.aliases.entry(alias.clone()).or_insert(original);
				*name = alias;
				*namespace = None;
			}
		}
		Ok(())
	}

	pub fn is_empty(&self) -> bool {
		self.aliases.is_empty()
	}

	pub fn restore_item(&self, item: &mut responses::OutputItem) {
		let (namespace, name) = match item {
			responses::OutputItem::FunctionCall(call) => (&mut call.namespace, &mut call.name),
			responses::OutputItem::CustomToolCall(call) => (&mut call.namespace, &mut call.name),
			_ => return,
		};
		if let Some(original) = self.aliases.get(name) {
			*namespace = Some(original.namespace.clone());
			name.clone_from(&original.name);
		}
	}

	pub fn restore_response(&self, response: &mut responses::Response) {
		for item in &mut response.output {
			self.restore_item(item);
		}
	}

	pub fn restore_event(&self, event: &mut responses::ResponseStreamEvent) {
		use responses::ResponseStreamEvent as Event;
		match event {
			Event::ResponseOutputItemAdded(event) => self.restore_item(&mut event.item),
			Event::ResponseOutputItemDone(event) => self.restore_item(&mut event.item),
			Event::ResponseCompleted(event) => self.restore_response(&mut event.response),
			Event::ResponseIncomplete(event) => self.restore_response(&mut event.response),
			Event::ResponseFailed(event) => self.restore_response(&mut event.response),
			Event::ResponseFunctionCallArgumentsDone(event) => {
				if let Some(original) = event.name.as_ref().and_then(|name| self.aliases.get(name)) {
					event.name = Some(original.name.clone());
				}
			},
			_ => {},
		}
	}
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn request_rewrite_errors_identify_the_problem() {
		for (input, expected) in [
			(
				json!({"tools": [{"type": "function", "name": "js"}, {"type": "function", "name": "js"}]}),
				"duplicate upstream tool name: js",
			),
			(
				json!({"tools": [
					{"type": "namespace", "name": "a__b", "description": "", "tools": [{"type": "function", "name": "c"}]},
					{"type": "namespace", "name": "a", "description": "", "tools": [{"type": "function", "name": "b__c"}]}
				]}),
				"duplicate upstream tool name: a__b__c",
			),
			(
				json!({"tools": [
					{"type": "function", "name": "kernel__js"},
					{"type": "namespace", "name": "kernel", "description": "", "tools": [{"type": "function", "name": "js"}]}
				]}),
				"duplicate upstream tool name: kernel__js",
			),
			(
				json!({"tools": [
				{"type": "namespace", "name": "one", "description": "", "tools": [{"type": "function", "name": "js"}]},
				{"type": "namespace", "name": "two", "description": "", "tools": [{"type": "function", "name": "js"}]}
			], "tool_choice": {"type": "function", "name": "js"}}),
				"ambiguous namespaced tool choice: js; use namespace__tool to select a member",
			),
			(
				json!({"tools": [
					{"type": "namespace", "name": "kernel", "description": "", "tools": [{"type": "function", "name": "js"}]}
				], "tool_choice": {"type": "custom", "name": "js"}}),
				"custom tool choice refers to a function tool: js",
			),
			(
				json!({"tools": [
					{"type": "namespace", "name": "kernel", "description": "", "tools": [{"type": "function", "name": "js"}]}
				], "tool_choice": {"type": "custom", "name": "kernel__js"}}),
				"custom tool choice refers to a function tool: kernel__js",
			),
			(
				json!({"tools": [{"type": "function", "name": "kernel__js"}],
					"input": [{"type": "function_call", "call_id": "call_1", "namespace": "kernel", "name": "js", "arguments": "{}"}]
				}),
				"history tool call collides with another tool: kernel__js",
			),
			(
				json!({"tool_choice": {"type": "allowed_tools", "mode": "auto", "tools": [{"type": "function", "name": "js"}]}}),
				"allowed_tools tool choice is unsupported for Chat Completions and Bedrock Converse",
			),
		] {
			let mut request = json!({"input": "hello"});
			request
				.as_object_mut()
				.unwrap()
				.extend(input.as_object().unwrap().clone());
			let mut request = serde_json::from_value(request).unwrap();
			assert!(
				matches!(NamespaceToolMap::rewrite_request(&mut request), Err(AIError::UnsupportedConversion(message)) if message.as_str() == expected),
				"{expected}"
			);
		}
	}

	#[test]
	fn choices_accept_unique_members_and_explicit_aliases() {
		for name in ["js", "kernel__js"] {
			let mut request = serde_json::from_value(json!({
				"input": [{"type": "function_call", "call_id": "call_1", "namespace": "", "name": "plain", "arguments": "{}"}],
				"tools": [{"type": "namespace", "name": "kernel", "description": "", "tools": [{"type": "function", "name": "js"}]}],
				"tool_choice": {"type": "function", "name": name}
			})).unwrap();
			NamespaceToolMap::rewrite_request(&mut request).unwrap();
			let request = serde_json::to_value(request).unwrap();
			assert_eq!(request["tool_choice"]["name"], "kernel__js");
			assert_eq!(request["input"][0]["namespace"], "");
		}
	}

	#[test]
	fn custom_members_keep_namespace_identity() {
		let mut request = serde_json::from_value(json!({
			"input": [{"type": "custom_tool_call", "id": "ctc_1", "call_id": "call_1", "namespace": "shell", "name": "exec", "input": "pwd"}],
			"tools": [
				{"type": "namespace", "name": "shell", "description": "Read-only commands", "tools": [
					{"type": "custom", "name": "exec", "description": "Run one command", "format": {"type": "text"}}
				]},
				{"type": "namespace", "name": "admin", "description": "Admin commands", "tools": [
					{"type": "custom", "name": "exec", "format": {"type": "text"}}
				]}
			],
			"tool_choice": {"type": "custom", "name": "shell__exec"}
		}))
		.unwrap();
		let namespaces = NamespaceToolMap::rewrite_request(&mut request).unwrap();
		let request = serde_json::to_value(request).unwrap();
		assert_eq!(request["tools"][0]["name"], "shell__exec");
		assert_eq!(
			request["tools"][0]["description"],
			"Read-only commands\n\nRun one command"
		);
		assert_eq!(request["tools"][1]["name"], "admin__exec");
		assert_eq!(request["tool_choice"]["name"], "shell__exec");
		assert_eq!(request["input"][0]["namespace"], serde_json::Value::Null);
		assert_eq!(request["input"][0]["name"], "shell__exec");

		let mut item = serde_json::from_value(json!({
			"type": "custom_tool_call",
			"call_id": "call_2",
			"id": "ctc_2",
			"name": "shell__exec",
			"input": "ls"
		}))
		.unwrap();
		namespaces.restore_item(&mut item);
		let item = serde_json::to_value(item).unwrap();
		assert_eq!(item["namespace"], "shell");
		assert_eq!(item["name"], "exec");
	}
}
