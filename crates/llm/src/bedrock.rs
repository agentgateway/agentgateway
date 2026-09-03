use agent_core::prelude::Strng;
use agent_core::strng;

use crate::apply;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum BedrockEndpointPreference {
	#[default]
	RuntimePreferred,
	MantlePreferred,
	MantleOnly,
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
	pub endpoint_preference: BedrockEndpointPreference,
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

	/// Resolves the endpoint for a route when the chat routes are correct
	pub fn resolve_endpoint(
		&self,
		route_type: super::RouteType,
		model_id: Option<&str>,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> BedrockEndpoint {
		use super::RouteType as RT;
		match route_type {
			// These routes are only served by the Runtime endpoint.
			RT::Embeddings | RT::GeminiCountTokens | RT::Rerank | RT::Realtime => {
				BedrockEndpoint::Runtime
			},
			// Model listing is a Mantle-native route.
			RT::Models => BedrockEndpoint::Mantle,
			// Passthrough/detect stay on Runtime; we cannot reason about the wire format.
			RT::Detect | RT::Passthrough | RT::GenerateContent => BedrockEndpoint::Runtime,
			// Chat, and Anthropic count-tokens, follow the model's endpoint: Runtime's Converse /
			// CountTokens APIs, or Mantle's native OpenAI/Anthropic APIs.
			RT::Completions | RT::Messages | RT::Responses | RT::AnthropicTokenCount => {
				self.chat_endpoint(model_id, catalog)
			},
		}
	}

	fn chat_endpoint(
		&self,
		model_id: Option<&str>,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> BedrockEndpoint {
		use BedrockEndpoint::{Mantle, Runtime};
		use BedrockEndpointPreference::*;

		use crate::model_catalog::tags;
		let has = |tag| model_id.is_some_and(|m| catalog.is_some_and(|c| c.model_has_tag(m, tag)));
		match self.endpoint_preference {
			RuntimeOnly => Runtime,
			MantleOnly => Mantle,
			// Prefer Runtime; use Mantle only for models tagged Mantle but not Runtime.
			RuntimePreferred if has(tags::MANTLE) && !has(tags::RUNTIME) => Mantle,
			RuntimePreferred => Runtime,
			// Prefer Mantle; use Runtime only for models tagged Runtime but not Mantle.
			MantlePreferred if has(tags::RUNTIME) && !has(tags::MANTLE) => Runtime,
			MantlePreferred => Mantle,
		}
	}

	/// SigV4 signing-service override for the route (`Some` for Mantle, else the default `bedrock`).
	pub fn signing_service_name(
		&self,
		route_type: super::RouteType,
		model_id: Option<&str>,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Option<&'static str> {
		match self.resolve_endpoint(route_type, model_id, catalog) {
			BedrockEndpoint::Mantle => Some(MANTLE_SIGNING_SERVICE_NAME),
			BedrockEndpoint::Runtime => None,
		}
	}

	/// Chat formats for `request_model`: Converse on Runtime; on Mantle, the tag-declared native formats (all three if untagged).
	pub fn supported_chat_formats(
		&self,
		request_model: Option<&str>,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Vec<super::ChatFormat> {
		use super::ChatFormat;
		match self.chat_endpoint(request_model, catalog) {
			BedrockEndpoint::Runtime => vec![ChatFormat::BedrockConverse],
			BedrockEndpoint::Mantle => {
				const NATIVE: [ChatFormat; 3] = [
					ChatFormat::OpenAICompletions,
					ChatFormat::AnthropicMessages,
					ChatFormat::OpenAIResponses,
				];
				if let Some(tags) = request_model.and_then(|m| catalog.and_then(|c| c.get_model_tags(m))) {
					let declared: Vec<ChatFormat> = NATIVE
						.into_iter()
						.filter(|f| tags.contains(f.tag()))
						.collect();
					if !declared.is_empty() {
						return declared;
					}
				}
				NATIVE.to_vec()
			},
		}
	}

	pub fn get_path_for_route(
		&self,
		route_type: super::RouteType,
		streaming: bool,
		model: &str,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Strng {
		if matches!(
			self.resolve_endpoint(route_type, Some(model), catalog),
			BedrockEndpoint::Mantle
		) {
			return match route_type {
				super::RouteType::Responses => strng::literal!("/v1/responses"),
				super::RouteType::Messages => strng::literal!("/anthropic/v1/messages"),
				super::RouteType::AnthropicTokenCount => {
					strng::literal!("/anthropic/v1/messages/count_tokens")
				},
				super::RouteType::Models => strng::literal!("/v1/models"),
				_ => strng::literal!("/v1/chat/completions"),
			};
		}

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

	pub fn get_host(
		&self,
		route_type: super::RouteType,
		model_id: Option<&str>,
		catalog: crate::model_catalog::Catalog<'_>,
	) -> Strng {
		// Rerank always uses the agent-runtime host, independent of endpoint choice.
		if matches!(route_type, super::RouteType::Rerank) {
			return strng::format!("bedrock-agent-runtime.{}.amazonaws.com", self.region);
		}
		match self.resolve_endpoint(route_type, model_id, catalog) {
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

	fn provider(pref: BedrockEndpointPreference) -> Provider {
		Provider {
			model: None,
			region: strng::new("us-east-1"),
			guardrail_identifier: None,
			guardrail_version: None,
			endpoint_preference: pref,
		}
	}

	#[test]
	fn resolve_endpoint_explicit_preferences_ignore_model_table() {
		let mantle = provider(BedrockEndpointPreference::MantleOnly);
		let runtime = provider(BedrockEndpointPreference::RuntimeOnly);
		assert_eq!(
			mantle.resolve_endpoint(RouteType::Messages, Some("any-model"), None),
			BedrockEndpoint::Mantle
		);
		assert_eq!(
			runtime.resolve_endpoint(RouteType::Messages, Some("any-model"), None),
			BedrockEndpoint::Runtime
		);
	}

	#[test]
	fn resolve_endpoint_runtime_preferred_routes_non_chat_by_route_type() {
		// These routes resolve by route type alone (no catalog lookup), so this is deterministic.
		let p = provider(BedrockEndpointPreference::RuntimePreferred);
		assert_eq!(
			p.resolve_endpoint(RouteType::Embeddings, None, None),
			BedrockEndpoint::Runtime
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::Rerank, None, None),
			BedrockEndpoint::Runtime
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::Models, None, None),
			BedrockEndpoint::Mantle
		);
	}

	#[test]
	fn resolve_endpoint_runtime_preferred_uses_catalog_tags() {
		use crate::model_catalog::{TestCatalog, tags};
		let p = provider(BedrockEndpointPreference::RuntimePreferred);
		let cat = TestCatalog::new([("openai.gpt-oss-120b", &[tags::MANTLE][..])]);
		let catalog: crate::model_catalog::Catalog = Some(&cat);
		assert_eq!(
			p.resolve_endpoint(RouteType::Completions, Some("openai.gpt-oss-120b"), catalog),
			BedrockEndpoint::Mantle
		);
		assert_eq!(
			p.resolve_endpoint(
				RouteType::Completions,
				Some("anthropic.claude-3-5-sonnet-20241022-v2:0"),
				catalog
			),
			BedrockEndpoint::Runtime
		);
	}

	#[test]
	fn resolve_endpoint_mantle_preferred_is_the_mirror_of_runtime_preferred() {
		use crate::model_catalog::{TestCatalog, tags};
		let p = provider(BedrockEndpointPreference::MantlePreferred);
		let cat = TestCatalog::new([
			("only.mantle", &[tags::MANTLE][..]),
			("only.runtime", &[tags::RUNTIME][..]),
			("both.endpoints", &[tags::MANTLE, tags::RUNTIME][..]),
		]);
		let catalog: crate::model_catalog::Catalog = Some(&cat);
		// Prefer Mantle: only a model tagged Runtime-but-not-Mantle falls back to Runtime.
		assert_eq!(
			p.resolve_endpoint(RouteType::Completions, Some("only.runtime"), catalog),
			BedrockEndpoint::Runtime
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::Completions, Some("only.mantle"), catalog),
			BedrockEndpoint::Mantle
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::Completions, Some("both.endpoints"), catalog),
			BedrockEndpoint::Mantle
		);
		// Untagged / no-catalog models default to Mantle under MantlePreferred.
		assert_eq!(
			p.resolve_endpoint(RouteType::Completions, Some("untagged.model"), catalog),
			BedrockEndpoint::Mantle
		);
		assert_eq!(
			p.resolve_endpoint(RouteType::Completions, Some("untagged.model"), None),
			BedrockEndpoint::Mantle
		);
		// Non-chat routes still resolve by route type regardless of the Mantle preference.
		assert_eq!(
			p.resolve_endpoint(RouteType::Embeddings, Some("only.mantle"), catalog),
			BedrockEndpoint::Runtime
		);
	}

	#[test]
	fn mantle_endpoint_uses_correct_host_path_and_signing() {
		let p = provider(BedrockEndpointPreference::MantleOnly);
		assert_eq!(
			p.get_host(RouteType::Messages, None, None).as_str(),
			"bedrock-mantle.us-east-1.api.aws"
		);
		assert_eq!(
			p.signing_service_name(RouteType::Messages, None, None),
			Some("bedrock-mantle")
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Messages, false, "m", None)
				.as_str(),
			"/anthropic/v1/messages"
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Responses, false, "m", None)
				.as_str(),
			"/v1/responses"
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Completions, false, "m", None)
				.as_str(),
			"/v1/chat/completions"
		);
	}

	#[test]
	fn runtime_endpoint_uses_correct_host_path_and_signing() {
		let p = provider(BedrockEndpointPreference::RuntimeOnly);
		assert_eq!(
			p.get_host(RouteType::Messages, None, None).as_str(),
			"bedrock-runtime.us-east-1.amazonaws.com"
		);
		assert_eq!(
			p.signing_service_name(RouteType::Messages, None, None),
			None
		);
		assert_eq!(
			p.get_path_for_route(
				RouteType::Messages,
				false,
				"anthropic.claude-3-5-haiku-20241022-v1:0",
				None
			)
			.as_str(),
			"/model/anthropic.claude-3-5-haiku-20241022-v1:0/converse"
		);
	}

	#[test]
	fn rerank_always_uses_agent_runtime_host() {
		for pref in [
			BedrockEndpointPreference::MantleOnly,
			BedrockEndpointPreference::RuntimeOnly,
			BedrockEndpointPreference::RuntimePreferred,
		] {
			assert_eq!(
				provider(pref)
					.get_host(RouteType::Rerank, None, None)
					.as_str(),
				"bedrock-agent-runtime.us-east-1.amazonaws.com"
			);
		}
	}

	#[test]
	fn supported_chat_formats_mantle_advertises_native_runtime_converse() {
		let mantle = provider(BedrockEndpointPreference::MantleOnly);
		assert_eq!(
			mantle.supported_chat_formats(Some("any"), None),
			vec![
				ChatFormat::OpenAICompletions,
				ChatFormat::AnthropicMessages,
				ChatFormat::OpenAIResponses,
			]
		);
		let runtime = provider(BedrockEndpointPreference::RuntimeOnly);
		assert_eq!(
			runtime.supported_chat_formats(Some("any"), None),
			vec![ChatFormat::BedrockConverse]
		);
	}

	#[test]
	fn non_chat_routes_stay_on_runtime_even_for_mantle_only() {
		// These routes exist only on Runtime, so MantleOnly must not force them to Mantle.
		let p = provider(BedrockEndpointPreference::MantleOnly);
		for rt in [
			RouteType::Embeddings,
			RouteType::Rerank,
			RouteType::Realtime,
			// Gemini-native routes are not served by Bedrock (Mantle has no generateContent /
			// count-tokens API), so they must not be forced onto Mantle either.
			RouteType::GenerateContent,
			RouteType::GeminiCountTokens,
		] {
			assert_eq!(
				p.resolve_endpoint(rt, Some("m"), None),
				BedrockEndpoint::Runtime,
				"{rt:?} must stay on Runtime under MantleOnly"
			);
		}
		// Host + path for embeddings must be the Runtime invoke path, not a Mantle path.
		assert_eq!(
			p.get_host(RouteType::Embeddings, Some("m"), None).as_str(),
			"bedrock-runtime.us-east-1.amazonaws.com"
		);
		assert_eq!(
			p.get_path_for_route(RouteType::Embeddings, false, "m", None)
				.as_str(),
			"/model/m/invoke"
		);
	}

	#[test]
	fn anthropic_count_tokens_follows_the_endpoint() {
		// Runtime uses the Bedrock CountTokens API; Mantle uses Anthropic's native count_tokens.
		let runtime = provider(BedrockEndpointPreference::RuntimeOnly);
		assert_eq!(
			runtime.resolve_endpoint(RouteType::AnthropicTokenCount, Some("m"), None),
			BedrockEndpoint::Runtime
		);
		assert_eq!(
			runtime
				.get_path_for_route(RouteType::AnthropicTokenCount, false, "m", None)
				.as_str(),
			"/model/m/count-tokens"
		);

		let mantle = provider(BedrockEndpointPreference::MantleOnly);
		assert_eq!(
			mantle.resolve_endpoint(RouteType::AnthropicTokenCount, Some("m"), None),
			BedrockEndpoint::Mantle
		);
		assert_eq!(
			mantle
				.get_host(RouteType::AnthropicTokenCount, Some("m"), None)
				.as_str(),
			"bedrock-mantle.us-east-1.api.aws"
		);
		assert_eq!(
			mantle.signing_service_name(RouteType::AnthropicTokenCount, Some("m"), None),
			Some("bedrock-mantle")
		);
		assert_eq!(
			mantle
				.get_path_for_route(RouteType::AnthropicTokenCount, false, "m", None)
				.as_str(),
			"/anthropic/v1/messages/count_tokens"
		);
	}
}
