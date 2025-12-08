pub mod from_completions {
	use super::helpers;
	use crate::json;
	use crate::llm::bedrock::Provider;
	use crate::llm::{anthropic, types, AIError};
	use itertools::Itertools;
	use std::collections::HashMap;
	use types::bedrock;
	use types::completions::typed as completions;

	/// translate an OpenAI completions request to a Bedrock converse  request
	pub fn translate(
		req: &types::completions::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, completions::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed, provider, headers, prompt_caching);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub(super) fn translate_internal(
		req: completions::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> bedrock::ConverseRequest {
		// Extract and join system prompts from completions format
		let system_text = req
			.messages
			.iter()
			.filter_map(|msg| {
				if completions::message_role(msg) == completions::SYSTEM_ROLE {
					completions::message_text(msg).map(|s| s.to_string())
				} else {
					None
				}
			})
			.collect::<Vec<String>>()
			.join("\n");

		let messages = req
			.messages
			.iter()
			.filter(|msg| completions::message_role(msg) != completions::SYSTEM_ROLE)
			.filter_map(|msg| {
				let role = match completions::message_role(msg) {
					completions::ASSISTANT_ROLE => bedrock::Role::Assistant,
					// Default to user for other roles
					_ => bedrock::Role::User,
				};

				completions::message_text(msg)
					.filter(|s| !s.trim().is_empty())
					.map(|s| vec![bedrock::ContentBlock::Text(s.to_string())])
					.map(|content| bedrock::Message { role, content })
			})
			.collect();

		let inference_config = bedrock::InferenceConfiguration {
			max_tokens: req.max_tokens(),
			temperature: req.temperature,
			top_p: req.top_p,
			// Map Anthropic-style vendor extension to Bedrock topK when provided
			top_k: req.vendor_extensions.top_k,
			stop_sequences: req.stop_sequence(),
		};

		// Build guardrail configuration if specified
		let guardrail_config = if let (Some(identifier), Some(version)) =
			(&provider.guardrail_identifier, &provider.guardrail_version)
		{
			Some(bedrock::GuardrailConfiguration {
				guardrail_identifier: identifier.to_string(),
				guardrail_version: version.to_string(),
				trace: Some("enabled".to_string()),
			})
		} else {
			None
		};

		// Build metadata from user field and x-bedrock-metadata header
		let mut metadata = req
			.user
			.map(|user| HashMap::from([("user_id".to_string(), user)]))
			.unwrap_or_default();

		// Extract metadata from x-bedrock-metadata header (set by ExtAuthz or transformation policy)
		if let Some(header_metadata) = super::helpers::extract_metadata_from_headers(headers) {
			metadata.extend(header_metadata);
		}

		let metadata = if metadata.is_empty() {
			None
		} else {
			Some(metadata)
		};

		let tool_choice = match req.tool_choice {
			Some(completions::ToolChoiceOption::Named(completions::NamedToolChoice {
				r#type: _,
				function,
			})) => Some(bedrock::ToolChoice::Tool {
				name: function.name,
			}),
			Some(completions::ToolChoiceOption::Auto) => Some(bedrock::ToolChoice::Auto),
			Some(completions::ToolChoiceOption::Required) => Some(bedrock::ToolChoice::Any),
			Some(completions::ToolChoiceOption::None) => None,
			None => None,
		};
		let tools = req.tools.map(|tools| {
			tools
				.into_iter()
				.map(|tool| {
					let tool_spec = bedrock::ToolSpecification {
						name: tool.function.name,
						description: tool.function.description,
						input_schema: tool.function.parameters.map(bedrock::ToolInputSchema::Json),
					};

					bedrock::Tool::ToolSpec(tool_spec)
				})
				.collect_vec()
		});
		let tool_config = tools.map(|tools| bedrock::ToolConfiguration { tools, tool_choice });

		// Handle thinking configuration similar to Anthropic
		let thinking = if let Some(budget) = req.vendor_extensions.thinking_budget_tokens {
			Some(serde_json::json!({
				"thinking": {
					"type": "enabled",
					"budget_tokens": budget
				}
			}))
		} else {
			match &req.reasoning_effort {
				// Note: Anthropic's minimum budget_tokens is 1024
				Some(completions::ReasoningEffort::Minimal) | Some(completions::ReasoningEffort::Low) => {
					Some(serde_json::json!({
						"thinking": {
							"type": "enabled",
							"budget_tokens": 1024
						}
					}))
				},
				Some(completions::ReasoningEffort::Medium) => Some(serde_json::json!({
					"thinking": {
						"type": "enabled",
						"budget_tokens": 2048
					}
				})),
				Some(completions::ReasoningEffort::High) => Some(serde_json::json!({
					"thinking": {
						"type": "enabled",
						"budget_tokens": 4096
					}
				})),
				None => None,
			}
		};

		let model_id = req.model.unwrap_or_default();
		let supports_caching = helpers::supports_prompt_caching(&model_id);
		let system_content = if system_text.is_empty() {
			None
		} else {
			let mut system_blocks = vec![bedrock::SystemContentBlock::Text { text: system_text }];
			tracing::debug!(
				"Prompt caching policy: {:?}, model: {}, supports caching: {}",
				prompt_caching.map(|c| (c.cache_system, c.cache_messages, c.cache_tools)),
				model_id,
				supports_caching
			);
			if let Some(caching) = prompt_caching
				&& caching.cache_system
				&& supports_caching
			{
				let meets_minimum = if let Some(min_tokens) = caching.min_tokens {
					helpers::estimate_system_tokens(&system_blocks) >= min_tokens
				} else {
					true
				};
				if meets_minimum {
					system_blocks.push(bedrock::SystemContentBlock::CachePoint {
						cache_point: helpers::create_cache_point(),
					});
				}
			}
			Some(system_blocks)
		};

		let mut bedrock_request = bedrock::ConverseRequest {
			model_id,
			messages,
			system: system_content,
			inference_config: Some(inference_config),
			tool_config,
			guardrail_config,
			additional_model_request_fields: thinking,
			prompt_variables: None,
			additional_model_response_field_paths: None,
			request_metadata: metadata,
			performance_config: None,
		};

		if let Some(caching) = prompt_caching {
			if caching.cache_messages && supports_caching {
				helpers::insert_cache_point_in_last_user_message(&mut bedrock_request.messages);
			}
			if caching.cache_tools
				&& supports_caching
				&& let Some(ref mut tool_config) = bedrock_request.tool_config
				&& !tool_config.tools.is_empty()
			{
				tool_config
					.tools
					.push(bedrock::Tool::CachePoint(helpers::create_cache_point()));
			}
		}

		bedrock_request
	}
}

pub mod from_messages {
	use super::helpers;
	use crate::json;
	use crate::llm::bedrock::Provider;
	use crate::llm::{types, AIError};
	use itertools::Itertools;
	use types::bedrock;
	use types::messages::typed as messages;

	/// translate an Anthropic messages request to a Bedrock converse request
	pub fn translate(
		req: &types::messages::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
	) -> Result<Vec<u8>, AIError> {
		let typed = json::convert::<_, messages::Request>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed, provider, headers);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub(super) fn translate_internal(
		req: messages::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
	) -> bedrock::ConverseRequest {
		let mut cache_points_used = 0;

		// Check if thinking is enabled (Bedrock constraint: thinking requires specific tool/temp settings)
		let thinking_enabled = req.thinking.is_some();

		// Convert system prompt to Bedrock format with cache point insertion
		// Note: Anthropic MessagesRequest.system is Option<SystemPrompt>, Bedrock wants Option<Vec<SystemContentBlock>>
		let system_content = req.system.as_ref().map(|sys| {
			let mut result = Vec::new();
			match sys {
				messages::SystemPrompt::Text(text) => {
					result.push(bedrock::SystemContentBlock::Text { text: text.clone() });
				},
				messages::SystemPrompt::Blocks(blocks) => {
					// Convert Anthropic system blocks to Bedrock system blocks with cache points
					for block in blocks {
						match block {
							messages::SystemContentBlock::Text {
								text,
								cache_control,
							} => {
								result.push(bedrock::SystemContentBlock::Text { text: text.clone() });
								// Insert cache point if this block has cache_control
								if cache_control.is_some() && cache_points_used < 4 {
									result.push(bedrock::SystemContentBlock::CachePoint {
										cache_point: helpers::create_cache_point(),
									});
									cache_points_used += 1;
								}
							},
						}
					}
				},
			}
			result
		});

		// Convert typed Anthropic messages to Bedrock messages
		let messages: Vec<bedrock::Message> = req
			.messages
			.into_iter()
			.map(|msg| {
				let role = match msg.role {
					messages::Role::Assistant => bedrock::Role::Assistant,
					messages::Role::User => bedrock::Role::User,
				};

				// Convert ContentBlocks from Anthropic → Bedrock, inserting cache points
				let mut content = Vec::with_capacity(msg.content.len() * 2);
				for block in msg.content {
					let (bedrock_block, has_cache_control) = match block {
						messages::ContentBlock::Text(messages::ContentTextBlock {
							text,
							cache_control,
							..
						}) => (bedrock::ContentBlock::Text(text), cache_control.is_some()),
						messages::ContentBlock::Image(messages::ContentImageBlock {
							source,
							cache_control,
						}) => {
							if let Some(media_type) = source.get("media_type").and_then(|v| v.as_str())
								&& let Some(data) = source.get("data").and_then(|v| v.as_str())
							{
								let format = media_type
									.strip_prefix("image/")
									.unwrap_or(media_type)
									.to_string();
								(
									bedrock::ContentBlock::Image(bedrock::ImageBlock {
										format,
										source: bedrock::ImageSource {
											bytes: data.to_string(),
										},
									}),
									cache_control.is_some(),
								)
							} else {
								continue;
							}
						},
						messages::ContentBlock::ToolUse {
							id,
							name,
							input,
							cache_control,
						} => (
							bedrock::ContentBlock::ToolUse(bedrock::ToolUseBlock {
								tool_use_id: id,
								name,
								input,
							}),
							cache_control.is_some(),
						),
						messages::ContentBlock::ToolResult {
							tool_use_id,
							content: tool_content,
							is_error,
							cache_control,
						} => {
							let bedrock_content = match tool_content {
								messages::ToolResultContent::Text(text) => {
									vec![bedrock::ToolResultContentBlock::Text(text)]
								},
								messages::ToolResultContent::Array(parts) => parts
									.into_iter()
									.filter_map(|part| match part {
										messages::ToolResultContentPart::Text { text, .. } => {
											Some(bedrock::ToolResultContentBlock::Text(text))
										},
										messages::ToolResultContentPart::Image { source, .. } => {
											if let Some(media_type) = source.get("media_type").and_then(|v| v.as_str())
												&& let Some(data) = source.get("data").and_then(|v| v.as_str())
											{
												let format = media_type
													.strip_prefix("image/")
													.unwrap_or(media_type)
													.to_string();
												Some(bedrock::ToolResultContentBlock::Image(
													bedrock::ImageBlock {
														format,
														source: bedrock::ImageSource {
															bytes: data.to_string(),
														},
													},
												))
											} else {
												None
											}
										},
										_ => None,
									})
									.collect(),
							};

							let status = is_error.map(|is_err| match is_err {
								true => bedrock::ToolResultStatus::Error,
								false => bedrock::ToolResultStatus::Success,
							});

							(
								bedrock::ContentBlock::ToolResult(bedrock::ToolResultBlock {
									tool_use_id,
									content: bedrock_content,
									status,
								}),
								cache_control.is_some(),
							)
						},
						messages::ContentBlock::Thinking {
							thinking,
							signature,
						} => (
							bedrock::ContentBlock::ReasoningContent(bedrock::ReasoningContentBlock::Structured {
								reasoning_text: bedrock::ReasoningText {
									text: thinking,
									signature: Some(signature),
								},
							}),
							false,
						),
						messages::ContentBlock::WebSearchToolResult { .. } => continue,
						messages::ContentBlock::RedactedThinking { .. } => continue,
						messages::ContentBlock::Document(_) => continue,
						messages::ContentBlock::SearchResult(_) => continue,
						messages::ContentBlock::ServerToolUse { .. } => continue,
						messages::ContentBlock::Unknown => continue,
					};

					content.push(bedrock_block);

					if has_cache_control && cache_points_used < 4 {
						content.push(bedrock::ContentBlock::CachePoint(
							helpers::create_cache_point(),
						));
						cache_points_used += 1;
					}
				}

				bedrock::Message { role, content }
			})
			.collect();

		// Build inference config from typed fields
		let inference_config = bedrock::InferenceConfiguration {
			max_tokens: req.max_tokens,
			// When thinking is enabled, temperature/top_p/top_k must be None (Bedrock constraint)
			temperature: if thinking_enabled {
				None
			} else {
				req.temperature
			},
			top_p: if thinking_enabled { None } else { req.top_p },
			top_k: if thinking_enabled { None } else { req.top_k },
			stop_sequences: req.stop_sequences,
		};

		// Convert typed tools to Bedrock tool config
		// NOTE: Only send toolConfig if we have at least one tool. Bedrock rejects empty tools arrays.
		let tool_config = if let Some(tools) = req.tools {
			let bedrock_tools: Vec<bedrock::Tool> = {
				let mut result = Vec::with_capacity(tools.len() * 2);
				for tool in tools {
					let has_cache_control = tool.cache_control.is_some();

					result.push(bedrock::Tool::ToolSpec(bedrock::ToolSpecification {
						name: tool.name,
						description: tool.description,
						input_schema: Some(bedrock::ToolInputSchema::Json(tool.input_schema)),
					}));

					if has_cache_control && cache_points_used < 4 {
						result.push(bedrock::Tool::CachePoint(helpers::create_cache_point()));
						cache_points_used += 1;
					}
				}
				result
			};

			if bedrock_tools.is_empty() {
				None
			} else {
				let tool_choice = match req.tool_choice {
					Some(messages::ToolChoice::Auto) => {
						if thinking_enabled {
							Some(bedrock::ToolChoice::Any)
						} else {
							Some(bedrock::ToolChoice::Auto)
						}
					},
					Some(messages::ToolChoice::Any) => Some(bedrock::ToolChoice::Any),
					Some(messages::ToolChoice::Tool { name }) => {
						if thinking_enabled {
							Some(bedrock::ToolChoice::Any)
						} else {
							Some(bedrock::ToolChoice::Tool { name })
						}
					},
					Some(messages::ToolChoice::None) | None => {
						if thinking_enabled {
							Some(bedrock::ToolChoice::Any)
						} else {
							None
						}
					},
				};

				Some(bedrock::ToolConfiguration {
					tools: bedrock_tools,
					tool_choice,
				})
			}
		} else {
			None
		};

		// Convert thinking from typed field and handle beta headers
		let mut additional_fields = req.thinking.map(|thinking| match thinking {
			messages::ThinkingInput::Enabled { budget_tokens } => serde_json::json!({
				"thinking": {
					"type": "enabled",
					"budget_tokens": budget_tokens
				}
			}),
			messages::ThinkingInput::Disabled {} => serde_json::json!({
				"thinking": {
					"type": "disabled"
				}
			}),
		});

		// Extract beta headers from HTTP headers if provided
		let beta_headers =
			headers.and_then(|h| crate::llm::bedrock::extract_beta_headers(h).ok().flatten());

		if let Some(beta_array) = beta_headers {
			// Add beta headers to additionalModelRequestFields
			match additional_fields {
				Some(ref mut fields) => {
					if let Some(existing_obj) = fields.as_object_mut() {
						existing_obj.insert(
							"anthropic_beta".to_string(),
							serde_json::Value::Array(beta_array),
						);
					}
				},
				None => {
					let mut fields = serde_json::Map::new();
					fields.insert(
						"anthropic_beta".to_string(),
						serde_json::Value::Array(beta_array),
					);
					additional_fields = Some(serde_json::Value::Object(fields));
				},
			}
		}

		// Build guardrail configuration if provider has it configured
		let guardrail_config = if let (Some(identifier), Some(version)) =
			(&provider.guardrail_identifier, &provider.guardrail_version)
		{
			Some(bedrock::GuardrailConfiguration {
				guardrail_identifier: identifier.to_string(),
				guardrail_version: version.to_string(),
				trace: Some("enabled".to_string()),
			})
		} else {
			None
		};

		// Build metadata from request field and x-bedrock-metadata header
		let mut metadata = req.metadata.map(|m| m.fields).unwrap_or_default();

		// Extract metadata from x-bedrock-metadata header (set by ExtAuthz or transformation policy)
		if let Some(header_metadata) = helpers::extract_metadata_from_headers(headers) {
			metadata.extend(header_metadata);
		}

		let metadata = if metadata.is_empty() {
			None
		} else {
			Some(metadata)
		};

		let bedrock_request = bedrock::ConverseRequest {
			model_id: req.model,
			messages,
			system: system_content,
			inference_config: Some(inference_config),
			tool_config,
			guardrail_config,
			additional_model_request_fields: additional_fields,
			prompt_variables: None,
			additional_model_response_field_paths: None,
			request_metadata: metadata,
			performance_config: None,
		};

		bedrock_request
	}
}

pub mod from_responses {
	use super::helpers;
	use crate::json;
	use crate::llm::bedrock::Provider;
	use crate::llm::{anthropic, types, AIError};
	use async_openai::types::responses::{
		ContentType, Input, InputContent, InputItem, InputMessage, ToolChoice, ToolChoiceMode,
		ToolDefinition,
	};
	use helpers::*;
	use itertools::Itertools;
	use types::bedrock;
	use types::responses::typed as responses;

	/// translate an OpenAI responses request to a Bedrock converse request
	pub fn translate(
		req: &types::responses::Request,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> Result<Vec<u8>, AIError> {
		let typed =
			json::convert::<_, responses::CreateResponse>(req).map_err(AIError::RequestMarshal)?;
		let xlated = translate_internal(typed, provider, headers, prompt_caching);
		serde_json::to_vec(&xlated).map_err(AIError::RequestMarshal)
	}

	pub(super) fn translate_internal(
		req: responses::CreateResponse,
		provider: &Provider,
		headers: Option<&http::HeaderMap>,
		prompt_caching: Option<&crate::llm::policy::PromptCachingConfig>,
	) -> bedrock::ConverseRequest {
		use crate::llm::openai::responses::{
			ContentType, Input, InputContent, InputItem, InputMessage, Role as ResponsesRole,
		};

		let supports_caching = crate::llm::bedrock::supports_prompt_caching(&req.model);

		// Convert input to Bedrock messages and system content
		let mut messages: Vec<bedrock::Message> = Vec::new();
		let mut system_blocks: Vec<bedrock::SystemContentBlock> = Vec::new();

		if let Ok(json) = serde_json::to_string_pretty(&req.input) {
			tracing::debug!("Converting Responses input to Bedrock: {}", json);
		}

		// Convert Input format to items
		let items = match &req.input {
			Input::Text(text) => {
				vec![InputItem::Message(InputMessage {
					kind: Default::default(),
					role: ResponsesRole::User,
					content: InputContent::TextInput(text.clone()),
				})]
			},
			Input::Items(items) => items.clone(),
		};

		// Process each input item
		for item in items {
			match item {
				InputItem::Message(msg) => {
					// Extract role and content
					let role = match msg.role {
						ResponsesRole::User => bedrock::Role::User,
						ResponsesRole::Assistant => bedrock::Role::Assistant,
						ResponsesRole::System | ResponsesRole::Developer => {
							// System and developer messages go to system array
							let text = match &msg.content {
								InputContent::TextInput(t) => t.clone(),
								InputContent::InputItemContentList(parts) => {
									// Extract text from content parts
									parts
										.iter()
										.filter_map(|part| match part {
											ContentType::InputText(input_text) => Some(input_text.text.clone()),
											_ => None,
										})
										.collect::<Vec<_>>()
										.join("\n")
								},
							};
							system_blocks.push(bedrock::SystemContentBlock::Text { text });
							continue;
						},
					};

					// Convert content to Bedrock content blocks
					let content = match &msg.content {
						InputContent::TextInput(text) => {
							vec![bedrock::ContentBlock::Text(text.clone())]
						},
						InputContent::InputItemContentList(parts) => {
							let mut blocks = Vec::new();
							tracing::debug!("Processing {} content parts", parts.len());
							for part in parts {
								match part {
									ContentType::InputText(input_text) => {
										tracing::debug!("Found InputText with text: {}", input_text.text);
										blocks.push(bedrock::ContentBlock::Text(input_text.text.clone()));
									},
									ContentType::InputImage(_) => {
										// Image support requires fetching URLs or resolving file_ids
										tracing::debug!("Image inputs not supported in Responses->Bedrock translation");
										continue;
									},
									ContentType::InputFile(_) => {
										tracing::debug!("Skipping InputFile");
										continue;
									},
								}
							}
							tracing::debug!("Created {} content blocks", blocks.len());
							blocks
						},
					};

					messages.push(bedrock::Message { role, content });
				},
				InputItem::Custom(custom_value) => {
					#[derive(serde::Deserialize)]
					struct CustomItem {
						#[serde(rename = "type")]
						item_type: Option<String>,
						call_id: Option<String>,
						name: Option<String>,
						arguments: Option<String>,
						output: Option<serde_json::Value>,
					}

					match serde_json::from_value::<CustomItem>(custom_value.clone()) {
						Ok(item) => {
							match item.item_type.as_deref() {
								Some("function_call") => {
									if let (Some(call_id), Some(name), Some(arguments)) =
										(item.call_id, item.name, item.arguments)
									{
										// Parse tool arguments, skip this tool call if JSON is invalid
										let Ok(input) = serde_json::from_str::<serde_json::Value>(&arguments) else {
											tracing::warn!(
												"Skipping function_call with invalid JSON arguments for tool '{}': {}",
												name,
												arguments
											);
											continue;
										};

										messages.push(bedrock::Message {
											role: bedrock::Role::Assistant,
											content: vec![bedrock::ContentBlock::ToolUse(bedrock::ToolUseBlock {
												tool_use_id: call_id,
												name,
												input,
											})],
										});
									}
								},
								Some("function_call_output") => {
									if let (Some(call_id), Some(output)) = (item.call_id, item.output) {
										let result_content = if let Some(output_str) = output.as_str() {
											vec![bedrock::ToolResultContentBlock::Text(
												output_str.to_string(),
											)]
										} else {
											let json_str = serde_json::to_string(&output).unwrap_or_default();
											vec![bedrock::ToolResultContentBlock::Text(json_str)]
										};

										messages.push(bedrock::Message {
											role: bedrock::Role::User,
											content: vec![bedrock::ContentBlock::ToolResult(
												bedrock::ToolResultBlock {
													tool_use_id: call_id,
													content: result_content,
													status: Some(bedrock::ToolResultStatus::Success),
												},
											)],
										});
									}
								},
								_ => {
									// Unknown custom type, skip
									tracing::warn!("Unknown custom input item type: {:?}", item.item_type);
									continue;
								},
							}
						},
						Err(e) => {
							tracing::warn!("Failed to parse custom input item: {}", e);
							continue;
						},
					}
				},
			}
		}

		let mut system_content = if system_blocks.is_empty() {
			None
		} else {
			Some(system_blocks)
		};

		// Add instructions field to system content if present
		if let Some(instructions) = &req.instructions {
			let instructions_block = bedrock::SystemContentBlock::Text {
				text: instructions.clone(),
			};
			if let Some(ref mut system) = system_content {
				system.insert(0, instructions_block);
			} else {
				system_content = Some(vec![instructions_block]);
			}
		}

		// Apply system prompt caching if configured
		if let Some(caching) = prompt_caching
			&& caching.cache_system
			&& supports_caching
			&& let Some(ref mut system) = system_content
		{
			let meets_minimum = if let Some(min_tokens) = caching.min_tokens {
				estimate_system_tokens(system) >= min_tokens
			} else {
				true
			};
			if meets_minimum {
				system.push(bedrock::SystemContentBlock::CachePoint {
					cache_point: create_cache_point(),
				});
			}
		}

		let inference_config = bedrock::InferenceConfiguration {
			max_tokens: req.max_output_tokens.unwrap_or(4096) as usize,
			temperature: req.temperature,
			top_p: req.top_p,
			top_k: None,
			stop_sequences: vec![],
		};

		// Convert tools from typed Responses API format to Bedrock format
		let (tools, tool_choice) = if let Some(response_tools) = &req.tools {
			let bedrock_tools: Vec<bedrock::Tool> = response_tools
				.iter()
				.filter_map(|tool_def| {
					use crate::llm::openai::responses::ToolDefinition;
					match tool_def {
						ToolDefinition::Function(func) => {
							Some(bedrock::Tool::ToolSpec(bedrock::ToolSpecification {
								name: func.name.clone(),
								description: func.description.clone(),
								input_schema: Some(bedrock::ToolInputSchema::Json(func.parameters.clone())),
							}))
						},
						_ => {
							tracing::warn!("Unsupported tool type in Responses API: {:?}", tool_def);
							None
						},
					}
				})
				.collect();

			let bedrock_tool_choice = req.tool_choice.as_ref().and_then(|tc| {
				use crate::llm::openai::responses::{ToolChoice, ToolChoiceMode};
				match tc {
					ToolChoice::Mode(ToolChoiceMode::Auto) => Some(bedrock::ToolChoice::Auto),
					ToolChoice::Mode(ToolChoiceMode::Required) => Some(bedrock::ToolChoice::Any),
					ToolChoice::Mode(ToolChoiceMode::None) => None,
					ToolChoice::Function { name } => Some(bedrock::ToolChoice::Tool { name: name.clone() }),
					ToolChoice::Hosted { .. } => {
						tracing::warn!("Hosted tool choice not supported for Bedrock");
						None
					},
				}
			});

			(bedrock_tools, bedrock_tool_choice)
		} else {
			(vec![], None)
		};

		let tool_config = if !tools.is_empty() {
			Some(bedrock::ToolConfiguration { tools, tool_choice })
		} else {
			None
		};

		let guardrail_config = if let (Some(identifier), Some(version)) =
			(&provider.guardrail_identifier, &provider.guardrail_version)
		{
			Some(bedrock::GuardrailConfiguration {
				guardrail_identifier: identifier.to_string(),
				guardrail_version: version.to_string(),
				trace: Some("enabled".to_string()),
			})
		} else {
			None
		};

		// Extract metadata from request body and merge with headers (consistent with Messages/Completions)
		let mut metadata = req.metadata.clone().unwrap_or_default();

		if let Some(header_metadata) = extract_metadata_from_headers(headers) {
			metadata.extend(header_metadata);
		}

		let metadata = if metadata.is_empty() {
			None
		} else {
			Some(metadata)
		};

		let mut bedrock_request = bedrock::ConverseRequest {
			model_id: req.model.clone(),
			messages,
			system: system_content,
			inference_config: Some(inference_config),
			tool_config,
			guardrail_config,
			additional_model_request_fields: None,
			prompt_variables: None,
			additional_model_response_field_paths: None,
			request_metadata: metadata,
			performance_config: None,
		};

		// Apply user message and tool caching
		if let Some(caching) = prompt_caching {
			if caching.cache_messages && supports_caching {
				insert_cache_point_in_last_user_message(&mut bedrock_request.messages);
			}
			if caching.cache_tools
				&& supports_caching
				&& let Some(ref mut tool_config) = bedrock_request.tool_config
				&& !tool_config.tools.is_empty()
			{
				tool_config
					.tools
					.push(bedrock::Tool::CachePoint(create_cache_point()));
			}
		}

		tracing::debug!(
			"Bedrock request - messages: {}, system blocks: {}, tools: {}, tool_choice: {:?}",
			bedrock_request.messages.len(),
			bedrock_request
				.system
				.as_ref()
				.map(|s| s.len())
				.unwrap_or(0),
			bedrock_request
				.tool_config
				.as_ref()
				.map(|tc| tc.tools.len())
				.unwrap_or(0),
			bedrock_request
				.tool_config
				.as_ref()
				.and_then(|tc| tc.tool_choice.as_ref())
		);

		bedrock_request
	}
}
mod helpers {
	use crate::llm::types::bedrock;
	use std::collections::HashMap;

	pub fn create_cache_point() -> bedrock::CachePointBlock {
		bedrock::CachePointBlock {
			r#type: bedrock::CachePointType::Default,
		}
	}

	pub fn supports_prompt_caching(model_id: &str) -> bool {
		let model_lower = model_id.to_lowercase();
		if model_lower.contains("anthropic.claude") {
			let excluded = ["claude-instant", "claude-v1", "claude-v2"];
			if excluded.iter().any(|pattern| model_lower.contains(pattern)) {
				return false;
			}
			return true;
		}
		if model_lower.contains("amazon.nova") {
			return true;
		}
		false
	}

	pub fn estimate_system_tokens(system: &[bedrock::SystemContentBlock]) -> usize {
		let word_count: usize = system
			.iter()
			.filter_map(|block| match block {
				bedrock::SystemContentBlock::Text { text } => Some(text.split_whitespace().count()),
				bedrock::SystemContentBlock::CachePoint { .. } => None,
			})
			.sum();
		(word_count * 13) / 10
	}

	pub fn insert_cache_point_in_last_user_message(messages: &mut [bedrock::Message]) {
		// Strategy: Cache everything BEFORE the last message (not including it)
		// This caches the conversation history but not the current turn's input
		//
		// Example:
		//   [User: "Hello", Assistant: "Hi", User: "How are you?"]
		//   Cache point goes after "Hi" (before current "How are you?")
		//
		// This way:
		//   - Conversation history: cached (cheap reads on subsequent turns)
		//   - Current input: full price (it's new each turn anyway)

		let len = messages.len();

		// If we have 0-1 messages, no point caching (nothing to reuse yet)
		if len < 2 {
			return;
		}

		// Insert cache point in the second-to-last message
		// This caches all history BEFORE the current turn
		let second_to_last_idx = len - 2;
		messages[second_to_last_idx]
			.content
			.push(bedrock::ContentBlock::CachePoint(create_cache_point()));

		tracing::debug!(
			"Inserted cachePoint before last message (in message at index {})",
			second_to_last_idx
		);
	}

	/// Extract metadata from x-bedrock-metadata header.
	/// Gateway operators can use CEL transformation to populate this header with extauthz data.
	pub fn extract_metadata_from_headers(
		headers: Option<&crate::http::HeaderMap>,
	) -> Option<HashMap<String, String>> {
		const BEDROCK_METADATA_HEADER: &str = "x-bedrock-metadata";

		let header_value = headers?.get(BEDROCK_METADATA_HEADER)?;
		let json_str = header_value.to_str().ok()?;
		let json = serde_json::from_str::<serde_json::Value>(json_str).ok()?;
		Some(extract_flat_metadata(&json))
	}

	/// Extract flat key-value pairs from JSON for Bedrock requestMetadata.
	/// Only extracts top-level primitive values (strings, numbers, booleans).
	pub fn extract_flat_metadata(value: &serde_json::Value) -> HashMap<String, String> {
		let mut metadata = HashMap::new();

		if let serde_json::Value::Object(obj) = value {
			for (key, val) in obj {
				match val {
					serde_json::Value::String(s) => {
						metadata.insert(key.clone(), s.clone());
					},
					serde_json::Value::Number(n) => {
						metadata.insert(key.clone(), n.to_string());
					},
					serde_json::Value::Bool(b) => {
						metadata.insert(key.clone(), b.to_string());
					},
					_ => {}, // Skip nested objects, arrays, null
				}
			}
		}

		metadata
	}
}
