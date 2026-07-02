use agent_core::prelude::Strng;
use agent_core::strng;

use crate::http::auth::aws::{AwsAssumeRoleCache, AwsCredentialsCache};
use crate::*;

#[derive(Debug, Clone)]
pub struct AwsRegion {
	pub region: String,
}

#[apply(schema!)]
pub enum BedrockProviderPreference {
	#[default]
	RuntimePreferred,
	MantleOnly,
	RuntimeOnly,
}

impl std::str::FromStr for BedrockProviderPreference {
	type Err = ();

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"RuntimePreferred" | "Runtime" => Ok(Self::RuntimePreferred),
			"MantleOnly" => Ok(Self::MantleOnly),
			"RuntimeOnly" => Ok(Self::RuntimeOnly),
			_ => Err(()),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BedrockEndpoint {
	Runtime,
	Mantle,
}

// TODO: find a better place for this
const MANTLE_SIGNING_SERVICE_NAME: &str = "bedrock-mantle";

const CRIS_GEO_PREFIXES: [&str; 4] = ["us-gov.", "us.", "eu.", "apac."];
fn strip_cris_prefix(model: &str) -> &str {
	for prefix in CRIS_GEO_PREFIXES {
		if let Some(stripped) = model.strip_prefix(prefix) {
			return stripped;
		}
	}
	model
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename = "BedrockProvider"))]
pub struct Provider {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub model: Option<Strng>, // Optional: model override for Bedrock API path
	pub region: Strng, // Required: AWS region
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_identifier: Option<Strng>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guardrail_version: Option<Strng>,
	#[serde(default)]
	pub provider_preference: BedrockProviderPreference,
	/// Per-provider AWS source credential cache, shared across requests via Arc.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub source_credentials_cache: AwsCredentialsCache,
	/// Per-provider AWS AssumeRole credential cache, shared across requests via Arc.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub assume_role_cache: AwsAssumeRoleCache,
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

	fn use_native_format(&self, request_model: Option<&str>) -> bool {
		matches!(
			self.provider_preference,
			BedrockProviderPreference::RuntimeOnly
		) && self.is_anthropic_model(request_model)
	}

	pub fn resolve_endpoint(
		&self,
		route_type: super::RouteType,
		model_id: Option<&str>,
	) -> BedrockEndpoint {
		use BedrockProviderPreference::*;

		match self.provider_preference {
			RuntimeOnly => return BedrockEndpoint::Runtime,
			MantleOnly => return BedrockEndpoint::Mantle,
			_ => {},
		}

		use super::RouteType as RT;
		match route_type {
			RT::Embeddings | RT::AnthropicTokenCount | RT::Rerank | RT::Realtime => {
				BedrockEndpoint::Runtime
			},
			RT::Models => BedrockEndpoint::Mantle,
			// these can be both mantle or not but not really sure for
			RT::Detect | RT::Passthrough => BedrockEndpoint::Runtime,

			RT::Completions | RT::Messages | RT::Responses => {
				if model_id
					.is_some_and(|m| super::bedrock_model_table::is_mantle_only(strip_cris_prefix(m)))
				{
					BedrockEndpoint::Mantle
				} else {
					BedrockEndpoint::Runtime
				}
			},
		}
	}

	// Mantle overrides the signing name which for aws auth currently defaults to bedrock
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

	pub fn routes_to_mantle(&self, input_format: super::InputFormat, model_id: Option<&str>) -> bool {
		let route_type = match input_format {
			super::InputFormat::Completions => super::RouteType::Completions,
			super::InputFormat::Messages => super::RouteType::Messages,
			super::InputFormat::Responses => super::RouteType::Responses,
			_ => return false,
		};
		matches!(
			self.resolve_endpoint(route_type, model_id),
			BedrockEndpoint::Mantle
		)
	}

	pub fn body_native_format(
		&self,
		input_format: super::InputFormat,
		request_model: &str,
	) -> Option<super::custom::ProviderFormat> {
		use super::custom::ProviderFormat;
		let route_type = match input_format {
			super::InputFormat::Completions => super::RouteType::Completions,
			super::InputFormat::Messages => super::RouteType::Messages,
			super::InputFormat::Responses => super::RouteType::Responses,
			_ => return None,
		};
		match self.resolve_endpoint(route_type, Some(request_model)) {
			BedrockEndpoint::Mantle => Some(match input_format {
				super::InputFormat::Completions => ProviderFormat::Completions,
				super::InputFormat::Messages => ProviderFormat::Messages,
				super::InputFormat::Responses => ProviderFormat::Responses,
				_ => unreachable!("route_type derived only from the three formats above"),
			}),
			BedrockEndpoint::Runtime => self
				.use_native_format(Some(request_model))
				.then_some(ProviderFormat::Messages),
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
			super::RouteType::Rerank => strng::literal!("/rerank"),
			_ if streaming => strng::format!("/model/{model}/converse-stream"),
			_ => strng::format!("/model/{model}/converse"),
		}
	}

	pub fn get_host(&self, route_type: super::RouteType, model_id: Option<&str>) -> Strng {
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
