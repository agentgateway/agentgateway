use std::collections::BTreeMap;

use agent_core::strng;

use crate::http::jwt::Claims;
use crate::llm::RequestType;
use crate::llm::policy::{Moderation, with_default_timeout};
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::{Backend, BackendTrafficPolicy, ResourceName};
use crate::*;

pub mod openai;

/// A provider-neutral moderation result for a single inspected item.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModerationVerdict {
	/// The provider's own overall decision, for the providers that report one. Mistral and
	/// bare OpenAI-compatible classifiers do not, which is why it is optional and why the
	/// categories below are the portable part.
	pub provider_flagged: Option<bool>,
	/// Category name to the provider's own boolean decision.
	pub categories: BTreeMap<Strng, bool>,
	/// Category name to score, for the providers that report them. Not yet read by the
	/// guard; carried so that publishing scores stays additive.
	pub scores: BTreeMap<Strng, f64>,
}

impl ModerationVerdict {
	/// The categories this verdict marks as violated, in a stable order.
	pub fn flagged_categories(&self) -> Vec<&str> {
		self
			.categories
			.iter()
			.filter(|(_, flagged)| **flagged)
			.map(|(category, _)| category.as_str())
			.collect()
	}
}

/// Whether a verdict should trigger the guard.
///
/// A single place on purpose: a future per-category threshold option becomes a parameter
/// here rather than a decision re-implemented by every provider.
///
/// The provider's own decision wins when it reports one, so the guard degrades closed. A
/// category the response type does not know about is dropped at deserialization — OpenAI
/// has added categories before — and scanning only the categories we can name would let
/// such a request through silently.
pub fn is_flagged(verdict: &ModerationVerdict) -> bool {
	verdict.provider_flagged.unwrap_or(false) || verdict.categories.values().any(|flagged| *flagged)
}

/// What differs between one moderation service and the next.
///
/// Everything else — URI composition, content type, JWT claims propagation, TLS, backend
/// policies, timeout, metrics labels, the flagged decision and the guard detail — lives in
/// [`send_request`] and is written once.
pub trait ModerationProvider: Send + Sync {
	/// Name of the synthetic backend this provider is called through, as it appears in
	/// outbound call metrics.
	fn resource_name(&self) -> Strng;
	/// Host used when the configuration does not override it.
	fn default_host(&self) -> Strng;
	/// Moderation model used when the configuration does not name one.
	fn default_model(&self) -> Strng;
	/// Build the request path and body. Never a whole HTTP request: the caller owns the
	/// URI so that a configurable target stays possible.
	fn build_request(
		&self,
		model: &str,
		messages: &[crate::llm::SimpleChatCompletionMessage],
	) -> anyhow::Result<(Strng, Vec<u8>)>;
	/// Read a response body into one verdict per inspected item.
	fn parse(&self, body: &[u8]) -> anyhow::Result<Vec<ModerationVerdict>>;
}

/// The provider a moderation guard is configured to call.
pub fn provider_for(_moderation: &Moderation) -> &'static dyn ModerationProvider {
	&openai::OpenAI
}

pub async fn send_request(
	req: &mut dyn RequestType,
	claims: Option<Claims>,
	client: &PolicyClient,
	moderation: &Moderation,
	provider: &dyn ModerationProvider,
) -> anyhow::Result<Vec<ModerationVerdict>> {
	let model = moderation
		.model
		.clone()
		.unwrap_or_else(|| provider.default_model());
	let messages = req.get_messages();
	let (path, body) = provider.build_request(model.as_str(), &messages)?;

	let mut pols = vec![BackendTrafficPolicy::BackendTLS(
		crate::http::backendtls::SYSTEM_TRUST.clone(),
	)];
	pols.extend(moderation.policies.iter().cloned());

	let mut rb = ::http::Request::builder()
		.uri(format!("https://{}{}", provider.default_host(), path))
		.method(::http::Method::POST)
		.header(::http::header::CONTENT_TYPE, "application/json");
	if let Some(claims) = claims {
		rb = rb.extension(claims);
	}
	let req = rb.body(crate::http::Body::from(body))?;
	let mock_be = Backend::Dynamic(
		ResourceName::new(provider.resource_name(), strng::literal!("")),
		None,
	);
	let resp = client
		.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Guardrail)
		.call_with_explicit_policies_list(with_default_timeout(req), mock_be, pols)
		.await?;
	let body = crate::http::read_resp_body(resp).await?;
	provider.parse(body.as_ref())
}
