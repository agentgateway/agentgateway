use std::collections::HashSet;
use std::hash::Hash;

use ::cel::Value;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serializer};
use subtle::ConstantTimeEq;

use crate::http::Request;
use crate::http::auth::AuthorizationLocation;
use crate::http::budget::{Budget, BudgetLimitUnit, MatchedBudgets, NANODOLLARS_PER_USD};
use crate::proxy::dtrace::{self, pol_result};
use crate::proxy::{ProxyError, ProxyResponse};
use crate::{apply, *};

#[cfg(test)]
#[path = "apikey_tests.rs"]
mod tests;

const TRACE_POLICY_KIND: &str = "api_key";

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("no API Key found")]
	Missing,

	#[error("invalid credentials")]
	InvalidCredentials,
}

/// Validation mode for API key authentication.
#[apply(schema!)]
#[cfg_attr(feature = "schema", schemars(rename = "APIKeyMode"))]
#[derive(Copy, PartialEq, Eq, Default)]
pub enum Mode {
	/// Require a valid API key.
	Strict,
	/// Validate the API key when present.
	/// This is the default option.
	/// Warning: this allows requests without an API key.
	#[default]
	Optional,
	/// Decode valid API keys for later policy use.
	/// Warning: this allows requests with missing or invalid API keys.
	Permissive,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")] // Intentionally NOT deny_unknown_fields since we use flatten
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(::cel::DynamicType)]
pub struct Claims {
	/// The API key value. Redacted by default; use `apiKey.key.unredacted()` to access the actual value.
	#[dynamic(with_value = "api_key_to_value")]
	pub key: APIKey,
	#[serde(default, flatten)]
	#[dynamic(flatten)]
	pub metadata: UserMetadata,
}

#[apply(schema!)]
pub struct APIKey(
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(serialize_with = "ser_redact", deserialize_with = "deser_key")]
	SecretString,
);

impl APIKey {
	pub fn new(s: impl Into<Box<str>>) -> Self {
		APIKey(SecretString::new(s.into()))
	}

	pub(crate) fn sha256(&self) -> APIKeyHash {
		APIKeyHash::from_raw_key(self.0.expose_secret())
	}
}

pub fn api_key_to_value<'a>(key: &'a APIKey) -> Value<'a> {
	crate::cel::secret_string_to_value(&key.0)
}

type UserMetadata = serde_json::Value;

impl Hash for APIKey {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.0.expose_secret().hash(state);
	}
}

impl PartialEq for APIKey {
	fn eq(&self, other: &Self) -> bool {
		// Use a constant-time comparison; a short-circuiting comparison would leak how many
		// leading bytes of a candidate key match a configured key through response timing.
		self
			.0
			.expose_secret()
			.as_bytes()
			.ct_eq(other.0.expose_secret().as_bytes())
			.into()
	}
}

impl Eq for APIKey {}

#[apply(schema!)]
#[derive(Hash, PartialEq, Eq)]
pub struct APIKeyHash(
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(serialize_with = "ser_key_hash", deserialize_with = "deser_key_hash")]
	String,
);

impl APIKeyHash {
	pub(crate) fn as_str(&self) -> &str {
		&self.0
	}

	pub fn from_raw_key(key: &str) -> Self {
		let digest = crate::crypto::digest::sha256(key.as_bytes());
		APIKeyHash(hex::encode(digest))
	}

	pub fn parse(key_hash: &str) -> Result<Self, String> {
		let Some(digest) = key_hash.strip_prefix("sha256:") else {
			return Err("keyHash must use the sha256:<hex> format".to_string());
		};
		let decoded = hex::decode(digest).map_err(|e| e.to_string())?;
		if decoded.len() != 32 {
			return Err("sha256 keyHash must decode to 32 bytes".to_string());
		}
		Ok(APIKeyHash(digest.to_ascii_lowercase()))
	}
}

fn deser_key_hash<'de, D>(deserializer: D) -> Result<String, D::Error>
where
	D: Deserializer<'de>,
{
	let input = String::deserialize(deserializer)?;
	APIKeyHash::parse(&input)
		.map(|hash| hash.0)
		.map_err(serde::de::Error::custom)
}

fn ser_key_hash<S>(digest: &str, serializer: S) -> Result<S::Ok, S::Error>
where
	S: Serializer,
{
	serializer.serialize_str(&format!("sha256:{digest}"))
}

#[apply(schema_ser!)]
pub struct APIKeyAuthentication {
	// A map of API keys to the policy for that key.
	#[serde(serialize_with = "ser_redact")]
	pub users: Arc<HashMap<APIKeyHash, APIKeyPolicy>>,

	/// Validation mode for API Key authentication
	pub mode: Mode,

	#[serde(default)]
	pub location: AuthorizationLocation,
}

#[derive(Debug, Clone)]
pub struct APIKeyPolicy {
	pub metadata: UserMetadata,
	pub allowed_models: AllowedModels,
	pub budgets: Option<MatchedBudgets>,
}

#[derive(Debug, Clone, Default)]
pub struct AllowedModels(Option<Vec<AllowedModelPattern>>);

#[derive(Debug, Clone, PartialEq, Eq)]
enum AllowedModelPattern {
	Exact(String),
	Prefix(String),
	Suffix(String),
	All,
}

impl AllowedModels {
	fn compile(patterns: Option<Vec<String>>) -> anyhow::Result<Self> {
		let Some(patterns) = patterns else {
			return Ok(Self(None));
		};
		if patterns.len() > 1 && patterns.iter().any(|pattern| pattern == "*") {
			anyhow::bail!("allowedModels cannot combine '*' with other values");
		}

		let mut compiled = Vec::with_capacity(patterns.len());
		for pattern in patterns {
			if pattern.is_empty() {
				anyhow::bail!("allowedModels cannot contain an empty model name");
			}
			let wildcard_count = pattern.bytes().filter(|byte| *byte == b'*').count();
			let pattern = match wildcard_count {
				0 => AllowedModelPattern::Exact(pattern),
				1 if pattern == "*" => AllowedModelPattern::All,
				1 if pattern.ends_with('*') => {
					AllowedModelPattern::Prefix(pattern.trim_end_matches('*').to_string())
				},
				1 if pattern.starts_with('*') => {
					AllowedModelPattern::Suffix(pattern.trim_start_matches('*').to_string())
				},
				_ => anyhow::bail!(
					"allowedModels pattern {pattern:?} must contain at most one wildcard, at the beginning or end"
				),
			};
			if !compiled.contains(&pattern) {
				compiled.push(pattern);
			}
		}
		Ok(Self(Some(compiled)))
	}

	pub fn allows(&self, model: &str) -> bool {
		self
			.0
			.as_ref()
			.is_none_or(|patterns| patterns.iter().any(|pattern| pattern.allows(model)))
	}
}

impl AllowedModelPattern {
	fn allows(&self, model: &str) -> bool {
		match self {
			Self::Exact(exact) => model == exact,
			Self::Prefix(prefix) => model.starts_with(prefix),
			Self::Suffix(suffix) => model.ends_with(suffix),
			Self::All => true,
		}
	}

	fn intersects(&self, configured_model: &str) -> bool {
		match self {
			Self::All => true,
			Self::Exact(model) => configured_model_pattern_matches(configured_model, model),
			Self::Prefix(allowed) => {
				if configured_model == "*" {
					return true;
				}
				if let Some(configured) = configured_model.strip_suffix('*') {
					return allowed.starts_with(configured) || configured.starts_with(allowed);
				}
				configured_model.starts_with('*') || configured_model.starts_with(allowed)
			},
			Self::Suffix(allowed) => {
				if configured_model == "*" {
					return true;
				}
				if let Some(configured) = configured_model.strip_prefix('*') {
					return allowed.ends_with(configured) || configured.ends_with(allowed);
				}
				configured_model.ends_with('*') || configured_model.ends_with(allowed)
			},
		}
	}
}

fn configured_model_pattern_matches(pattern: &str, model: &str) -> bool {
	if pattern == "*" {
		return true;
	}
	if let Some(prefix) = pattern.strip_suffix('*') {
		return model.starts_with(prefix);
	}
	if let Some(suffix) = pattern.strip_prefix('*') {
		return model.ends_with(suffix);
	}
	pattern == model
}

#[derive(Debug, Clone)]
pub struct ModelAccessPolicy {
	allowed_models: AllowedModels,
}

impl ModelAccessPolicy {
	pub fn allows(&self, model: &str) -> bool {
		self.allowed_models.allows(model)
	}
}

pub fn discoverable_models<'a>(
	policy: Option<&'a ModelAccessPolicy>,
	configured_model: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
	let patterns = policy.and_then(|policy| policy.allowed_models.0.as_deref());
	let configured_is_pattern = configured_model.contains('*');
	let configured = std::iter::once(configured_model).filter(move |_| {
		if configured_is_pattern {
			patterns.is_none()
		} else {
			patterns.is_none_or(|patterns| {
				patterns
					.iter()
					.any(|pattern| pattern.allows(configured_model))
			})
		}
	});
	let mut emitted_configured = false;
	let intersections = patterns.into_iter().flatten().filter_map(move |pattern| {
		if !configured_is_pattern {
			return None;
		}
		match pattern {
			AllowedModelPattern::Exact(model)
				if configured_model_pattern_matches(configured_model, model) =>
			{
				Some(model.as_str())
			},
			pattern if pattern.intersects(configured_model) && !emitted_configured => {
				emitted_configured = true;
				Some(configured_model)
			},
			_ => None,
		}
	});
	configured.chain(intersections)
}

struct AuthenticatedAPIKey {
	claims: Claims,
	model_access: ModelAccessPolicy,
	budgets: Option<MatchedBudgets>,
}

impl APIKeyAuthentication {
	pub fn new(
		keys: impl IntoIterator<Item = (APIKey, UserMetadata)>,
		mode: Mode,
		location: AuthorizationLocation,
	) -> Self {
		Self {
			users: Arc::new(
				keys
					.into_iter()
					.map(|(key, metadata)| {
						(
							key.sha256(),
							APIKeyPolicy {
								metadata,
								allowed_models: AllowedModels::default(),
								budgets: None,
							},
						)
					})
					.collect(),
			),
			mode,
			location,
		}
	}
	async fn verify(&self, req: &mut Request) -> Result<Option<AuthenticatedAPIKey>, ProxyError> {
		let Some(key) = self.location.extract(req) else {
			// In strict mode, we require credentials
			if self.mode == Mode::Strict {
				pol_result!(
					dtrace::Error,
					Apply,
					"rejected request because API key is required but missing"
				);
				return Err(ProxyError::APIKeyAuthenticationFailure(Error::Missing));
			}
			// Otherwise without credentials, don't attempt to authenticate
			pol_result!(
				dtrace::Info,
				Skip,
				"request has no API key and auth mode is not strict"
			);
			return Ok(None);
		};

		let key = APIKey::new(key);
		if let Some(policy) = self.users.get(&key.sha256()) {
			pol_result!(
				dtrace::Info,
				Apply,
				"authenticated request with API key with metadata {}",
				serde_json::to_string(&policy.metadata).unwrap_or_default()
			);
			Ok(Some(AuthenticatedAPIKey {
				claims: Claims {
					key,
					metadata: policy.metadata.clone(),
				},
				model_access: ModelAccessPolicy {
					allowed_models: policy.allowed_models.clone(),
				},
				budgets: policy.budgets.clone(),
			}))
		} else if self.mode == Mode::Permissive {
			pol_result!(
				dtrace::Warn,
				Skip,
				"API key verification failed, continue due to permissive mode"
			);
			Ok(None)
		} else {
			pol_result!(
				dtrace::Error,
				Apply,
				"rejected request because API key credentials are invalid"
			);
			Err(ProxyError::APIKeyAuthenticationFailure(
				Error::InvalidCredentials,
			))
		}
	}
}

impl crate::store::RequestPolicyTrait for APIKeyAuthentication {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut Request,
	) -> Result<crate::http::PolicyResponse, ProxyResponse> {
		let res = self.verify(req).await.map_err(ProxyResponse::from)?;
		if let Some(authenticated) = res {
			self.location.remove(req).map_err(ProxyResponse::from)?;
			// Insert the claims into extensions so we can reference it later
			req.extensions_mut().insert(authenticated.claims);
			req.extensions_mut().insert(authenticated.model_access);
			if let Some(budgets) = authenticated.budgets {
				req.extensions_mut().insert(budgets);
			}
		}
		Ok(crate::http::PolicyResponse::default())
	}

	fn expressions(&self) -> impl Iterator<Item = &crate::cel::Expression> {
		self.location.expression().into_iter()
	}
}

#[apply(schema_de!)]
pub struct LocalAPIKeys {
	/// API keys that are accepted by this policy.
	pub keys: Vec<LocalAPIKey>,

	/// Controls whether requests must include a valid API key.
	#[serde(default)]
	pub mode: Mode,

	/// Where to read the API key from in incoming requests.
	#[serde(default)]
	pub location: AuthorizationLocation,
}

#[apply(schema_de!)]
#[serde(untagged)]
pub enum LocalAPIKey {
	Key {
		/// API key value to accept.
		key: APIKey,
		/// Optional metadata attached to requests authenticated with this key.
		metadata: Option<UserMetadata>,
		/// Model patterns this key is allowed to access.
		/// Omitted means no additional constraint; an empty list denies all models.
		#[serde(rename = "allowedModels", default)]
		allowed_models: Option<Vec<String>>,
		/// Independent budgets charged after LLM responses. A request is not charged when its provider
		/// does not report the usage or cost required by the budget unit.
		#[serde(default)]
		budgets: Vec<Budget>,
	},
	Sha256 {
		/// SHA-256 hash of an API key value to accept, in `sha256:<hex>` format.
		#[serde(rename = "keyHash")]
		key_hash: APIKeyHash,
		/// Optional metadata attached to requests authenticated with this key.
		metadata: Option<UserMetadata>,
		/// Model patterns this key is allowed to access.
		/// Omitted means no additional constraint; an empty list denies all models.
		#[serde(rename = "allowedModels", default)]
		allowed_models: Option<Vec<String>>,
		/// Independent budgets charged after LLM responses. A request is not charged when its provider
		/// does not report the usage or cost required by the budget unit.
		#[serde(default)]
		budgets: Vec<Budget>,
	},
}

impl LocalAPIKey {
	fn into_parts(self) -> anyhow::Result<(APIKeyHash, APIKeyPolicy)> {
		let (key_hash, metadata, allowed_models, budgets) = match self {
			LocalAPIKey::Key {
				key,
				metadata,
				allowed_models,
				budgets,
			} => (key.sha256(), metadata, allowed_models, budgets),
			LocalAPIKey::Sha256 {
				key_hash,
				metadata,
				allowed_models,
				budgets,
			} => (key_hash, metadata, allowed_models, budgets),
		};
		let metadata = metadata.unwrap_or_default();
		let api_key = metadata
			.get("name")
			.and_then(serde_json::Value::as_str)
			.filter(|name| !name.is_empty())
			.map(str::to_owned);
		if !budgets.is_empty() && api_key.is_none() {
			anyhow::bail!("API keys with budgets must have a metadata.name");
		}
		let mut budget_names = HashSet::new();
		for budget in &budgets {
			anyhow::ensure!(!budget.name.is_empty(), "budget names must not be empty");
			anyhow::ensure!(
				budget_names.insert(&budget.name),
				"duplicate budget name {:?} on API key {:?}",
				budget.name,
				api_key.as_deref().unwrap_or_default(),
			);
			let window_ms = budget.window.rolling.as_millis();
			anyhow::ensure!(
				window_ms > 0,
				"budget rolling windows must be greater than zero"
			);
			anyhow::ensure!(
				window_ms <= i64::MAX as u128,
				"budget rolling window is too large"
			);
			let amount = budget.limit.amount.decimal().normalize();
			let multiplier = match budget.limit.unit {
				BudgetLimitUnit::Usd => {
					anyhow::ensure!(
						amount.scale() <= 9,
						"USD budget limits support at most 9 fractional digits"
					);
					NANODOLLARS_PER_USD
				},
				BudgetLimitUnit::Tokens => {
					anyhow::ensure!(
						amount.fract().is_zero(),
						"token budget limits must be whole numbers"
					);
					1
				},
			};
			anyhow::ensure!(
				amount * Decimal::from(multiplier) <= Decimal::from(i64::MAX),
				"budget limit exceeds database integer range"
			);
		}
		let budgets = (!budgets.is_empty()).then(|| MatchedBudgets {
			api_key: api_key.expect("budget API keys have a name"),
			api_key_id: key_hash.as_str().to_owned(),
			budgets,
		});
		Ok((
			key_hash,
			APIKeyPolicy {
				metadata,
				allowed_models: AllowedModels::compile(allowed_models)?,
				budgets,
			},
		))
	}
}

impl LocalAPIKeys {
	pub fn compile(self) -> anyhow::Result<APIKeyAuthentication> {
		Ok(APIKeyAuthentication {
			users: Arc::new(
				self
					.keys
					.into_iter()
					.map(LocalAPIKey::into_parts)
					.collect::<anyhow::Result<_>>()?,
			),
			mode: self.mode,
			location: self.location,
		})
	}

	pub fn into(self) -> APIKeyAuthentication {
		self
			.compile()
			.expect("API key allowedModels configuration must be valid")
	}
}
