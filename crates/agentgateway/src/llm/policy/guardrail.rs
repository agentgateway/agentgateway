//! Typed backend for AI guardrail provider integrations.
//!
//! A guardrail backend owns the *connection* configuration for a guardrail
//! provider: the gateway derives the endpoint host, TLS, and implicit auth from
//! the provider type, and the user supplies only the instance-specific details
//! (e.g. the Bedrock guardrail identifier/version/region). Because it is a real,
//! store-resident backend, `BackendPolicy` (auth, timeouts, health) can be
//! attached to it and the outbound call is visible in telemetry under the
//! backend's name.
//!
//! Guardrail backends are referenced from prompt guard policies via
//! `backendRef`; they are never route targets. Per-request *behavior* (rejection
//! response, analysis thresholds) stays on the guard so one backend can be
//! reused across routes with different behavior.

use agent_core::prelude::Strng;
use agent_core::strng;

use crate::http::auth::{AwsAuth, AzureAuth, BackendAuth, GcpAuth};
use crate::store::BackendPolicies;
use crate::types::agent::Target;
use crate::*;

/// A typed guardrail integration backend, with one variant per supported provider.
#[apply(schema!)]
pub enum GuardrailBackend {
	/// AWS Bedrock Guardrails (ApplyGuardrail API)
	Bedrock(GuardrailBedrock),
	/// Google Cloud Model Armor
	GoogleModelArmor(GuardrailGoogleModelArmor),
	/// Azure AI Content Safety
	AzureContentSafety(GuardrailAzureContentSafety),
	/// OpenAI moderations API
	#[serde(rename = "openAIModeration")]
	OpenAIModeration(GuardrailOpenAIModeration),
}

#[apply(schema!)]
pub struct GuardrailBedrock {
	/// The unique identifier of the guardrail
	pub identifier: Strng,
	/// The version of the guardrail
	pub version: Strng,
	/// AWS region where the guardrail is deployed (e.g. "us-west-2"). Used to construct the endpoint host.
	pub region: Strng,
}

#[apply(schema!)]
pub struct GuardrailGoogleModelArmor {
	/// The template ID for the Model Armor configuration
	pub template_id: Strng,
	/// The GCP project ID
	pub project_id: Strng,
	/// The GCP region, used to construct the endpoint host (default: us-central1)
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub location: Option<Strng>,
}

#[apply(schema!)]
pub struct GuardrailAzureContentSafety {
	/// The Azure resource name, used to construct the endpoint host
	/// (`<resourceName>.cognitiveservices.azure.com`).
	/// Exactly one of resourceName and endpoint must be set.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub resource_name: Option<Strng>,
	/// The full endpoint host or URL (e.g. "https://<resource-name>.cognitiveservices.azure.com"),
	/// for endpoints whose host cannot be derived from the resource name.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub endpoint: Option<Strng>,
	/// Cached implicit Azure auth credential, shared across requests.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	pub cached_auth: AzureAuth,
}

#[apply(schema!)]
pub struct GuardrailOpenAIModeration {}

impl GuardrailGoogleModelArmor {
	pub fn location(&self) -> &str {
		self
			.location
			.as_deref()
			.unwrap_or(DEFAULT_MODEL_ARMOR_LOCATION)
	}
}

impl GuardrailAzureContentSafety {
	pub fn validate(&self) -> anyhow::Result<()> {
		match (&self.resource_name, &self.endpoint) {
			(Some(_), Some(_)) => {
				anyhow::bail!("azureContentSafety: only one of resourceName and endpoint may be set")
			},
			(None, None) => {
				anyhow::bail!("azureContentSafety: one of resourceName or endpoint must be set")
			},
			_ => Ok(()),
		}
	}

	fn host(&self) -> Strng {
		if let Some(rn) = &self.resource_name {
			return strng::format!("{}.cognitiveservices.azure.com", rn);
		}
		// Accept a bare host or a URL; strip the scheme and any trailing slash.
		let endpoint = self.endpoint.as_deref().unwrap_or_default();
		let endpoint = endpoint.trim_end_matches('/');
		let host = endpoint
			.strip_prefix("https://")
			.or_else(|| endpoint.strip_prefix("http://"))
			.unwrap_or(endpoint);
		strng::new(host)
	}
}

const DEFAULT_MODEL_ARMOR_LOCATION: &str = "us-central1";

impl GuardrailBackend {
	/// The provider endpoint host, derived from the instance configuration.
	pub fn host(&self) -> Strng {
		match self {
			GuardrailBackend::Bedrock(b) => {
				strng::format!("bedrock-runtime.{}.amazonaws.com", b.region)
			},
			GuardrailBackend::GoogleModelArmor(g) => {
				strng::format!("modelarmor.{}.rep.googleapis.com", g.location())
			},
			GuardrailBackend::AzureContentSafety(a) => a.host(),
			GuardrailBackend::OpenAIModeration(_) => crate::llm::openai::DEFAULT_HOST,
		}
	}

	pub fn target(&self) -> Target {
		Target::Hostname(self.host(), 443)
	}

	/// Gateway-owned transport defaults: system TLS plus the provider's implicit
	/// auth. These are fallbacks; policies attached to the backend take precedence.
	pub fn default_policies(&self) -> BackendPolicies {
		let backend_auth = match self {
			GuardrailBackend::Bedrock(_) => Some(BackendAuth::Aws(AwsAuth::Implicit {
				service_name: None,
				assume_role: None,
				source_credentials_cache: Default::default(),
				assume_role_cache: Default::default(),
			})),
			GuardrailBackend::GoogleModelArmor(_) => Some(BackendAuth::Gcp(GcpAuth::default())),
			GuardrailBackend::AzureContentSafety(a) => Some(BackendAuth::Azure(a.cached_auth.clone())),
			// No implicit auth for OpenAI; the user must attach an auth policy.
			GuardrailBackend::OpenAIModeration(_) => None,
		};
		BackendPolicies {
			backend_tls: Some(crate::http::backendtls::SYSTEM_TRUST.clone()),
			backend_auth,
			..Default::default()
		}
	}

	pub fn validate(&self) -> anyhow::Result<()> {
		match self {
			GuardrailBackend::AzureContentSafety(a) => a.validate(),
			_ => Ok(()),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn azure(resource_name: Option<&str>, endpoint: Option<&str>) -> GuardrailBackend {
		GuardrailBackend::AzureContentSafety(GuardrailAzureContentSafety {
			resource_name: resource_name.map(strng::new),
			endpoint: endpoint.map(strng::new),
			cached_auth: Default::default(),
		})
	}

	#[test]
	fn test_host_derivation() {
		let b = GuardrailBackend::Bedrock(GuardrailBedrock {
			identifier: strng::new("id"),
			version: strng::new("1"),
			region: strng::new("us-west-2"),
		});
		assert_eq!(b.host(), "bedrock-runtime.us-west-2.amazonaws.com");

		let m = GuardrailBackend::GoogleModelArmor(GuardrailGoogleModelArmor {
			template_id: strng::new("t"),
			project_id: strng::new("p"),
			location: None,
		});
		assert_eq!(m.host(), "modelarmor.us-central1.rep.googleapis.com");
		let m = GuardrailBackend::GoogleModelArmor(GuardrailGoogleModelArmor {
			template_id: strng::new("t"),
			project_id: strng::new("p"),
			location: Some(strng::new("europe-west1")),
		});
		assert_eq!(m.host(), "modelarmor.europe-west1.rep.googleapis.com");

		assert_eq!(
			azure(Some("my-resource"), None).host(),
			"my-resource.cognitiveservices.azure.com"
		);
		assert_eq!(
			azure(
				None,
				Some("https://my-resource.cognitiveservices.azure.com/")
			)
			.host(),
			"my-resource.cognitiveservices.azure.com"
		);
		assert_eq!(
			azure(None, Some("my-resource.cognitiveservices.azure.com")).host(),
			"my-resource.cognitiveservices.azure.com"
		);

		let o = GuardrailBackend::OpenAIModeration(GuardrailOpenAIModeration {});
		assert_eq!(o.host(), "api.openai.com");
	}

	#[test]
	fn test_azure_validation() {
		assert!(azure(Some("rn"), None).validate().is_ok());
		assert!(azure(None, Some("host")).validate().is_ok());
		assert!(azure(Some("rn"), Some("host")).validate().is_err());
		assert!(azure(None, None).validate().is_err());
	}

	#[test]
	fn test_deserialize() {
		let b: GuardrailBackend = crate::serdes::yamlviajson::from_str(
			r#"
bedrock:
  identifier: my-guardrail
  version: DRAFT
  region: us-west-2
"#,
		)
		.unwrap();
		let GuardrailBackend::Bedrock(b) = b else {
			panic!("expected bedrock provider");
		};
		assert_eq!(b.identifier, "my-guardrail");
		assert_eq!(b.version, "DRAFT");
		assert_eq!(b.region, "us-west-2");

		let m: GuardrailBackend = crate::serdes::yamlviajson::from_str("openAIModeration: {}").unwrap();
		assert!(matches!(m, GuardrailBackend::OpenAIModeration(_)));
	}
}
