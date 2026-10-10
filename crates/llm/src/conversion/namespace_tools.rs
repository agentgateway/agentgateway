use std::collections::{HashMap, HashSet};

use agent_core::strng;
use async_openai::types::responses::{FunctionTool, NamespaceToolParamTool};

use crate::AIError;
use crate::types::responses::typed as responses;

pub(crate) const NAMESPACE_SEPARATOR: &str = "__";

#[derive(Debug, Clone, PartialEq, Eq)]
struct OriginalTool {
	namespace: String,
	name: String,
}

/// Request-local aliases
#[derive(Debug, Clone, Default)]
pub struct NamespaceToolMap {
	aliases: HashMap<String, OriginalTool>,
}

impl NamespaceToolMap {
	/// Rewrite namespace definitions, forced function choices, and function-call history
	/// for Chat Completions and Bedrock Converse. Returns aliases for response restoration.
	/// Bare choices must identify a unique member; qualified `namespace__function` names
	/// are also accepted. Custom namespace members and allowed-tool constraints are unsupported.
	/// On error the request may be partially rewritten and must be discarded.
	pub fn rewrite_request(req: &mut responses::CreateResponse) -> Result<Self, AIError> {
		let mut map = Self::default();
		map.flatten_tools(&mut req.tools)?;
		let mut names = HashSet::new();
		for tool in req.tools.iter().flatten() {
			if let responses::Tool::Function(function) = tool
				&& !names.insert(function.name.as_str())
			{
				return Err(AIError::UnsupportedConversion(strng::format!(
					"duplicate upstream tool name: {}",
					function.name
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
					let NamespaceToolParamTool::Function(function) = member else {
						return Err(AIError::UnsupportedConversion(strng::literal!(
							"namespaced custom tools cannot be converted to function tools"
						)));
					};
					let name = format!("{}{NAMESPACE_SEPARATOR}{}", namespace.name, function.name);

					self.aliases.insert(
						name.clone(),
						OriginalTool {
							namespace: namespace.name.clone(),
							name: function.name,
						},
					);
					// Keep the namespace's instructions visible after removing its container.
					let mut description = function.description;
					if !namespace.description.is_empty() {
						let mut combined = namespace.description.clone();
						if let Some(member) = description.as_ref().filter(|text| !text.is_empty()) {
							combined.push_str("\n\n");
							combined.push_str(member);
						}
						description = Some(combined);
					}
					tools.push(responses::Tool::Function(FunctionTool {
						name,
						description,
						parameters: function.parameters,
						strict: function.strict,
						defer_loading: function.defer_loading,
						allowed_callers: function.allowed_callers,
						output_schema: function.output_schema,
						r#async: function.r#async,
					}));
				}
			}
		}

		Ok(())
	}

	fn rewrite_choice(
		&self,
		choice: &mut Option<responses::ToolChoiceParam>,
		names: &HashSet<&str>,
	) -> Result<(), AIError> {
		// Neither target conversion can enforce an allowed-tools constraint.
		if matches!(choice, Some(responses::ToolChoiceParam::AllowedTools(_))) {
			return Err(AIError::UnsupportedConversion(strng::literal!(
				"allowed_tools tool choice is unsupported for Chat Completions and Bedrock Converse"
			)));
		}

		if let Some(responses::ToolChoiceParam::Function(choice)) = choice
			&& !names.contains(choice.name.as_str())
		{
			let mut matches = self
				.aliases
				.iter()
				.filter(|(_, original)| original.name == choice.name);
			match (matches.next(), matches.next()) {
				(Some((alias, _)), None) => choice.name = alias.clone(),
				(Some(_), Some(_)) => {
					return Err(AIError::UnsupportedConversion(strng::format!(
						"ambiguous namespaced tool choice: {}; use namespace__function to select a member",
						choice.name
					)));
				},
				_ => {},
			}
		}

		Ok(())
	}

	fn rewrite_history(
		&mut self,
		input: &mut responses::InputParam,
		names: &HashSet<&str>,
	) -> Result<(), AIError> {
		if let responses::InputParam::Items(items) = input {
			for item in items {
				let responses::InputItem::Item(responses::Item::FunctionCall(call)) = item else {
					continue;
				};
				let Some(namespace) = call.namespace.as_ref().filter(|ns| !ns.is_empty()) else {
					continue;
				};
				let alias = format!("{namespace}{NAMESPACE_SEPARATOR}{}", call.name);
				let original = OriginalTool {
					namespace: namespace.clone(),
					name: call.name.clone(),
				};
				let collides = match self.aliases.get(&alias) {
					Some(existing) => existing != &original,
					None => names.contains(alias.as_str()),
				};
				if collides {
					return Err(AIError::UnsupportedConversion(strng::format!(
						"history function call collides with another tool: {alias}"
					)));
				}
				self.aliases.insert(alias.clone(), original);
				call.name = alias;
				call.namespace = None;
			}
		}
		Ok(())
	}

	pub fn is_empty(&self) -> bool {
		self.aliases.is_empty()
	}

	pub fn restore_item(&self, item: &mut responses::OutputItem) {
		if let responses::OutputItem::FunctionCall(call) = item
			&& let Some(original) = self.aliases.get(&call.name)
		{
			call.namespace = Some(original.namespace.clone());
			call.name = original.name.clone();
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
				"ambiguous namespaced tool choice: js; use namespace__function to select a member",
			),
			(
				json!({"tools": [{"type": "function", "name": "kernel__js"}],
					"input": [{"type": "function_call", "call_id": "call_1", "namespace": "kernel", "name": "js", "arguments": "{}"}]
				}),
				"history function call collides with another tool: kernel__js",
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
}
