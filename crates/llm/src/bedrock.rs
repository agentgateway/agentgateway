use agent_core::prelude::Strng;
use agent_core::strng;

use crate::apply;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

/// Which Bedrock endpoint to prefer when a model is not explicitly known to be Runtime.
#[apply(schema_enum!)]
#[derive(Default)]
pub enum BedrockProviderPreference {
	/// Prefer Runtime; fall through to Mantle only for chat models on the allow-list.
	#[default]
	RuntimePreferred,
	/// Always use Mantle for chat routes.
	MantleOnly,
	/// Always use Runtime for chat routes.
	RuntimeOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockEndpoint {
	Runtime,
	Mantle,
}

// Mantle signs SigV4 under this name instead of the default "bedrock".
const MANTLE_SIGNING_SERVICE_NAME: &str = "bedrock-mantle";

#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "BedrockProviderConfig"))]
pub struct Provider {
	/// Model ID to send to Bedrock, overriding the model in the client request.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // Optional: model override for Bedrock API path
	/// AWS region for the Bedrock endpoint.
	pub region: Strng, // Required: AWS region
	/// Identifier of the Bedrock guardrail to apply.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,
	/// Version of the Bedrock guardrail to apply.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,
	/// Which endpoint to prefer (Runtime vs Mantle).
	#[serde(default)]
	pub provider_preference: BedrockProviderPreference,
}

impl super::Provider for Provider {
	const NAME: Strng = strng::literal!("aws.bedrock");
}

impl Provider {
	pub fn is_anthropic_model(&self, request_model: Option<&str>) -> bool {
		let model = self
			.model
			.as_deref()
			.or(request_model)
			.unwrap_or_default()
			.to_ascii_lowercase();
		model.contains("anthropic.claude")
	}

	/// Resolves the endpoint for a route: non-chat routes are Runtime-only; chat routes honor the preference (and the allow-list for `RuntimePreferred`).
	pub fn resolve_endpoint(
		&self,
		route_type: super::RouteType,
		model_id: Option<&str>,
	) -> BedrockEndpoint {
		use super::RouteType as RT;
		match route_type {
			// These routes are only served by the Runtime endpoint.
			RT::Embeddings | RT::AnthropicTokenCount | RT::Rerank | RT::Realtime => {
				return BedrockEndpoint::Runtime;
			},
			// Model listing is a Mantle-native route.
			RT::Models => return BedrockEndpoint::Mantle,
			// Passthrough/detect stay on Runtime; we cannot reason about the wire format.
			RT::Detect | RT::Passthrough => return BedrockEndpoint::Runtime,
			RT::Completions | RT::Messages | RT::Responses => {},
		}

		use BedrockProviderPreference::*;
		match self.provider_preference {
			RuntimeOnly => BedrockEndpoint::Runtime,
			MantleOnly => BedrockEndpoint::Mantle,
			// Mantle only if the effective model (provider override, else request) is on the allow-list.
			RuntimePreferred => {
				let effective = self.model.as_deref().or(model_id);
				if effective.is_some_and(crate::bedrock_model_table::is_mantle_only) {
					BedrockEndpoint::Mantle
				} else {
					BedrockEndpoint::Runtime
				}
			},
		}
	}

	/// SigV4 signing-service override for the route (`Some` for Mantle, else the default `bedrock`).
	pub fn signing_service_name(
		&self,
		route_type: super::RouteType,
		model_id: Option<&str>,
	) -> Option<&'static str> {
		match self.resolve_endpoint(route_type, model_id) {
			BedrockEndpoint::Mantle => Some(MANTLE_SIGNING_SERVICE_NAME),
			BedrockEndpoint::Runtime => None,
		}
	}

	/// Chat formats for `request_model`: native passthrough on Mantle, Converse on Runtime.
	pub fn supported_chat_formats(&self, request_model: Option<&str>) -> Vec<super::ChatFormat> {
		use super::ChatFormat;
		// Completions is representative; all chat routes resolve the same for a model.
		match self.resolve_endpoint(super::RouteType::Completions, request_model) {
			BedrockEndpoint::Mantle => vec![
				ChatFormat::OpenAICompletions,
				ChatFormat::AnthropicMessages,
				ChatFormat::OpenAIResponses,
			],
			BedrockEndpoint::Runtime => vec![ChatFormat::BedrockConverse],
		}
	}

	pub fn get_path_for_route(
		&self,
		route_type: super::RouteType,
		streaming: bool,
		model: &str,
	) -> Strng {
		if matches!(
			self.resolve_endpoint(route_type, Some(model)),
			BedrockEndpoint::Mantle
		) {
			return match route_type {
				super::RouteType::Responses => strng::literal!("/v1/responses"),
				super::RouteType::Messages => strng::literal!("/anthropic/v1/messages"),
				super::RouteType::Models => strng::literal!("/v1/models"),
				_ => strng::literal!("/v1/chat/completions"),
			};
		}

		let model = self.model.as_deref().unwrap_or(model);
		const MODEL_SEGMENT: &percent_encoding::AsciiSet =
			&percent_encoding::CONTROLS.add(b'/').add(b'%');
		let model = percent_encoding::utf8_percent_encode(model, MODEL_SEGMENT);
		match route_type {
			super::RouteType::AnthropicTokenCount => strng::format!("/model/{model}/count-tokens"),
			super::RouteType::Embeddings => strng::format!("/model/{model}/invoke"),
			// Rerank uses the agent-runtime Rerank action (model goes in the body as an ARN).
			super::RouteType::Rerank => strng::literal!("/rerank"),
			_ if streaming => strng::format!("/model/{model}/converse-stream"),
			_ => strng::format!("/model/{model}/converse"),
		}
	}

	pub fn get_host(&self, route_type: super::RouteType, model_id: Option<&str>) -> Strng {
		// Rerank always uses the agent-runtime host, independent of endpoint choice.
		if matches!(route_type, super::RouteType::Rerank) {
			return strng::format!("bedrock-agent-runtime.{}.amazonaws.com", self.region);
		}
		match self.resolve_endpoint(route_type, model_id) {
			BedrockEndpoint::Mantle => strng::format!("bedrock-mantle.{}.api.aws", self.region),
			BedrockEndpoint::Runtime => {
				strng::format!("bedrock-runtime.{}.amazonaws.com", self.region)
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{ChatFormat, RouteType};

	fn provider(pref: BedrockProviderPreference) -> Provider {
		Provider {
			model: None,
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
			provider_preference: pref,
		}
	}

	#[test]
	fn resolve_endpoint_explicit_preferences_ignore_model_table() {
		let mantle = provider(BedrockProviderPreference::MantleOnly);
		let runtime = provider(BedrockProviderPreference::RuntimeOnly);
		assert_eq!(
			mantle.resolve_endpoint(RouteType::Messages, Some("any-model")),
			BedrockEndpoint::Mantle
		);
		assert_eq!(
			runtime.resolve_endpoint(RouteType::Messages, Some("any-model")),
			BedrockEndpoint::Runtime
		);
	}

	#[test]
	fn resolve_endpoint_runtime_preferred_routes_non_chat_by_route_type() {
		// These routes don't consult the global model table, so this is deterministic.
		let p = provider(BedrockProviderPreference::RuntimePreferred);
		assert_eq!(
			p.resolve_endpoint(RouteType::Embeddings, None),
			BedrockEndpoint::Runtime
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::Rerank, None),
			BedrockEndpoint::Runtime
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::AnthropicTokenCount, None),
			BedrockEndpoint::Runtime
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::Models, None),
			BedrockEndpoint::Mantle
		);
	}

	#[test]
	fn resolve_endpoint_runtime_preferred_uses_allow_list() {
		let _lock = crate::bedrock_model_table::MODELS_LOCK.lock().unwrap();
		let p = provider(BedrockProviderPreference::RuntimePreferred);
		crate::bedrock_model_table::set_mantle_models(["openai.gpt-oss-120b".to_string()].into());
		assert_eq!(
			p.resolve_endpoint(RouteType::Completions, Some("openai.gpt-oss-120b")),
			BedrockEndpoint::Mantle
		);
		assert_eq!(
			p.resolve_endpoint(
				RouteType::Completions,
				Some("anthropic.claude-3-5-sonnet-20241022-v2:0")
			),
			BedrockEndpoint::Runtime
		);
		crate::bedrock_model_table::restore_default();
	}

	#[test]
	fn mantle_endpoint_uses_correct_host_path_and_signing() {
		let p = provider(BedrockProviderPreference::MantleOnly);
		assert_eq!(
			p.get_host(RouteType::Messages, None).as_str(),
			"bedrock-mantle.us-east-1.api.aws"
		);
		assert_eq!(
			p.signing_service_name(RouteType::Messages, None),
			Some("bedrock-mantle")
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Messages, false, "m")
				.as_str(),
			"/anthropic/v1/messages"
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Responses, false, "m")
				.as_str(),
			"/v1/responses"
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Completions, false, "m")
				.as_str(),
			"/v1/chat/completions"
		);
	}

	#[test]
	fn runtime_endpoint_uses_correct_host_path_and_signing() {
		let p = provider(BedrockProviderPreference::RuntimeOnly);
		assert_eq!(
			p.get_host(RouteType::Messages, None).as_str(),
			"bedrock-runtime.us-east-1.amazonaws.com"
		);
		assert_eq!(p.signing_service_name(RouteType::Messages, None), None);
		assert_eq!(
			p.get_path_for_route(
				RouteType::Messages,
				false,
				"anthropic.claude-3-5-haiku-20241022-v1:0"
			)
			.as_str(),
			"/model/anthropic.claude-3-5-haiku-20241022-v1:0/converse"
		);
	}

	#[test]
	fn rerank_always_uses_agent_runtime_host() {
		for pref in [
			BedrockProviderPreference::MantleOnly,
			BedrockProviderPreference::RuntimeOnly,
			BedrockProviderPreference::RuntimePreferred,
		] {
			assert_eq!(
				provider(pref).get_host(RouteType::Rerank, None).as_str(),
				"bedrock-agent-runtime.us-east-1.amazonaws.com"
			);
		}
	}

	#[test]
	fn supported_chat_formats_mantle_advertises_native_runtime_converse() {
		let mantle = provider(BedrockProviderPreference::MantleOnly);
		assert_eq!(
			mantle.supported_chat_formats(Some("any")),
			vec![
				ChatFormat::OpenAICompletions,
				ChatFormat::AnthropicMessages,
				ChatFormat::OpenAIResponses,
			]
		);
		let runtime = provider(BedrockProviderPreference::RuntimeOnly);
		assert_eq!(
			runtime.supported_chat_formats(Some("any")),
			vec![ChatFormat::BedrockConverse]
		);
	}

	#[test]
	fn non_chat_routes_stay_on_runtime_even_for_mantle_only() {
		// These routes exist only on Runtime, so MantleOnly must not force them to Mantle.
		let p = provider(BedrockProviderPreference::MantleOnly);
		for rt in [
			RouteType::Embeddings,
			RouteType::Rerank,
			RouteType::AnthropicTokenCount,
			RouteType::Realtime,
		] {
			assert_eq!(
				p.resolve_endpoint(rt, Some("m")),
				BedrockEndpoint::Runtime,
				"{rt:?} must stay on Runtime under MantleOnly"
			);
		}
		// Host + path for embeddings must be the Runtime invoke path, not a Mantle path.
		assert_eq!(
			p.get_host(RouteType::Embeddings, Some("m")).as_str(),
			"bedrock-runtime.us-east-1.amazonaws.com"
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Embeddings, false, "m")
				.as_str(),
			"/model/m/invoke"
		);
	}
}
