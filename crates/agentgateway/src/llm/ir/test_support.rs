//! Fixtures shared by gateway IR subsystem and end-to-end tests.

use std::sync::Arc;

use serde_json::Value;

use crate::cel;
use crate::llm::policy::Policy;
use crate::llm::{AIProvider, NamedAIProvider, openai};
use crate::store::LLMRequestPolicies;

pub(super) fn openai(model_override: Option<&str>) -> AIProvider {
	AIProvider::OpenAI(openai::Provider {
		model_override: model_override.map(Into::into),
		moderation: None,
	})
}

pub(super) fn named(provider: AIProvider) -> NamedAIProvider {
	NamedAIProvider {
		name: "default".into(),
		provider,
		provider_backend: None,
		host_override: None,
		path_override: None,
		path_prefix: None,
		tokenize: false,
		inline_policies: Vec::new(),
	}
}

pub(super) fn with_policy(policy: Policy) -> LLMRequestPolicies {
	LLMRequestPolicies {
		llm: Some(Arc::new(policy)),
		..Default::default()
	}
}

pub(super) fn from_json<T: serde::de::DeserializeOwned>(value: Value) -> T {
	serde_json::from_value(value).unwrap()
}

pub(super) fn expr(expression: &str) -> Arc<cel::Expression> {
	Arc::new(cel::Expression::new_strict(expression).unwrap())
}
