use agent_core::strng;
use agent_core::strng::Strng;

use crate::llm::RouteType;
use crate::*;

#[apply(schema!)]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("openai");
}
pub const DEFAULT_HOST_STR: &str = "api.openai.com";
pub const DEFAULT_HOST: Strng = strng::literal!(DEFAULT_HOST_STR);

pub fn path(route: RouteType) -> &'static str {
	match route {
		// For Responses we forward to the responses endpoint
		RouteType::Responses => "/v1/responses",
		// For Embeddings we forward to the embeddings endpoint
		RouteType::Embeddings => "/v1/embeddings",
		// All others get translated down to completions
		_ => "/v1/chat/completions",
	}
}

pub mod responses {
	// Re-export async-openai Responses API types for cleaner usage
	pub use async_openai::types::responses::{
		Content, ContentType, CreateResponse, FunctionCall, Input, InputContent, InputItem,
		InputMessage, OutputContent, OutputMessage, OutputStatus, OutputText, ResponseEvent, Role,
		ToolChoice, ToolChoiceMode, ToolDefinition,
	};
	pub async fn process_streaming(
		log: crate::telemetry::log::AsyncLog<crate::llm::LLMInfo>,
		resp: crate::http::Response,
	) -> crate::http::Response {
		let buffer_limit = crate::http::response_buffer_limit(&resp);
		let mut saw_token = false;

		resp.map(|b| {
			crate::parse::sse::json_passthrough::<ResponseEvent>(b, buffer_limit, move |event| {
				let Some(Ok(event)) = event else {
					return;
				};

				match event {
					ResponseEvent::ResponseCreated(created) => {
						log.non_atomic_mutate(|r| {
							if let Some(model) = &created.response.model {
								r.response.provider_model = Some(agent_core::strng::new(model));
							}
							if let Some(usage) = &created.response.usage {
								r.response.input_tokens = Some(usage.input_tokens as u64);
								r.response.output_tokens = Some(usage.output_tokens as u64);
								r.response.total_tokens = Some(usage.total_tokens as u64);
							}
						});
					},
					ResponseEvent::ResponseOutputTextDelta(_) => {
						if !saw_token {
							saw_token = true;
							log.non_atomic_mutate(|r| {
								r.response.first_token = Some(std::time::Instant::now());
							});
						}
					},
					ResponseEvent::ResponseCompleted(completed) => {
						log.non_atomic_mutate(|r| {
							if let Some(model) = &completed.response.model {
								r.response.provider_model = Some(agent_core::strng::new(model));
							}
							if let Some(usage) = &completed.response.usage {
								r.response.input_tokens = Some(usage.input_tokens as u64);
								r.response.output_tokens = Some(usage.output_tokens as u64);
								r.response.total_tokens = Some(usage.total_tokens as u64);
							}
						});
					},
					_ => {
						// Ignore other events
					},
				}
			})
		})
	}
}
