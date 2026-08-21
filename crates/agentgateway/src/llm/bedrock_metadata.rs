//! Operator-resolved Bedrock request metadata.
//!
//! Amazon Bedrock accepts per-call key/value metadata on `InvokeModel`,
//! `InvokeModelWithResponseStream`, `Converse`, and `ConverseStream`. The
//! values are recorded in the model invocation logs (not on the bill), so they
//! are the per-prompt attribution layer: which user, app, or feature a specific
//! call belonged to, queryable in CloudWatch Logs Insights or Athena.
//!
//! Bedrock does not enforce metadata: a request that omits it succeeds, and
//! AWS's own guidance is to set it in a shared client or LLM gateway. This
//! module lets the operator do that with the same shape as STS session tags
//! (`assumeRole.tags`): a static `value`, or a CEL `expression` evaluated
//! against the request, for example `jwt.sub` or `request.headers["x-app"]`.
//! Resolution fails closed: an expression that cannot produce a valid value
//! rejects the request before it reaches Bedrock.
//!
//! Operator entries take precedence over any caller-supplied metadata
//! (the `x-bedrock-metadata` escape hatch): a key the operator claims cannot be
//! overridden by the caller, matching how session tags and pinned attribution
//! values behave elsewhere in the gateway.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::llm::AIError;
use crate::*;

/// Maximum number of metadata entries Bedrock accepts per request.
pub const MAX_REQUEST_METADATA_ENTRIES: usize = 16;
/// Maximum length of a metadata key or value, in characters.
pub const MAX_REQUEST_METADATA_LEN: usize = 256;

/// Header carrying request metadata on the `InvokeModel` family of APIs. It is
/// SigV4-signed like every other `x-amzn-*` header the gateway sends.
pub const REQUEST_METADATA_HEADER: &str = "x-amzn-bedrock-request-metadata";
/// Body field carrying request metadata on `Converse` and `ConverseStream`.
pub const REQUEST_METADATA_FIELD: &str = "requestMetadata";

/// Bedrock documents "a restricted set of alphanumeric and punctuation
/// characters" without enumerating it. This is the STS tag character set,
/// which is the conservative subset shared with session tags, so a value that
/// is valid as a tag is valid here too.
static ALLOWED_CHARS: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^[\p{L}\p{Z}\p{N}_.:/=+\-@]+$").expect("static regex"));

/// One request-metadata entry in configuration form. Exactly one of `value`
/// and `expression` must be set.
#[apply(schema!)]
pub struct BedrockRequestMetadataEntry {
	/// Metadata key.
	pub key: String,
	/// Static metadata value.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub value: Option<String>,
	/// CEL expression evaluated against each request to produce the value, for
	/// example `jwt.sub` or `request.headers["x-app"]`. If the expression does not
	/// produce a valid value at request time, the request is rejected.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expression: Option<Arc<cel::Expression>>,
}

/// Request metadata in runtime form: static values pre-validated, dynamic (CEL)
/// values compiled and evaluated per request.
#[derive(Debug, Clone, Default)]
pub struct BedrockRequestMetadata {
	// Sorted by key so serialization and tests are deterministic.
	static_entries: Arc<[(String, String)]>,
	dynamic_entries: Arc<[(String, Arc<cel::Expression>)]>,
}

impl PartialEq for BedrockRequestMetadata {
	fn eq(&self, other: &Self) -> bool {
		self.static_entries == other.static_entries
			&& self.dynamic_entries.len() == other.dynamic_entries.len()
			&& self
				.dynamic_entries
				.iter()
				.zip(other.dynamic_entries.iter())
				.all(|((ka, ea), (kb, eb))| ka == kb && ea.original_expression == eb.original_expression)
	}
}

impl BedrockRequestMetadata {
	/// Validates and splits configured entries into static and dynamic sets.
	/// Everything Bedrock would reject that can be checked without a request is
	/// checked here, so config errors surface at load time rather than per call.
	pub fn try_new(entries: Vec<BedrockRequestMetadataEntry>) -> anyhow::Result<Self> {
		if entries.len() > MAX_REQUEST_METADATA_ENTRIES {
			anyhow::bail!(
				"at most {MAX_REQUEST_METADATA_ENTRIES} request metadata entries are allowed, got {}",
				entries.len()
			);
		}
		let mut keys = HashSet::with_capacity(entries.len());
		let mut static_entries = Vec::new();
		let mut dynamic_entries = Vec::new();
		for entry in entries {
			validate_key(&entry.key)?;
			if !keys.insert(entry.key.clone()) {
				anyhow::bail!("duplicate request metadata key {:?}", entry.key);
			}
			match (entry.value, entry.expression) {
				(Some(value), None) => {
					validate_value(&entry.key, &value)?;
					static_entries.push((entry.key, value));
				},
				(None, Some(expression)) => dynamic_entries.push((entry.key, expression)),
				_ => anyhow::bail!(
					"request metadata {:?} must set exactly one of 'value' or 'expression'",
					entry.key
				),
			}
		}
		static_entries.sort();
		dynamic_entries.sort_by(|(a, _), (b, _)| a.cmp(b));
		Ok(Self {
			static_entries: static_entries.into(),
			dynamic_entries: dynamic_entries.into(),
		})
	}

	pub fn is_empty(&self) -> bool {
		self.static_entries.is_empty() && self.dynamic_entries.is_empty()
	}

	/// Expressions to register with the CEL context so the request snapshot
	/// captures what they read (JWT claims, headers).
	pub fn expressions(&self) -> impl Iterator<Item = &cel::Expression> {
		self.dynamic_entries.iter().map(|(_, e)| e.as_ref())
	}

	/// Evaluates dynamic entries and merges them with the static ones. Fails
	/// closed: an expression that cannot produce a valid value is an error.
	pub fn resolve(&self, exec: &cel::Executor<'_>) -> Result<Vec<(String, String)>, AIError> {
		let mut resolved: Vec<(String, String)> =
			Vec::with_capacity(self.static_entries.len() + self.dynamic_entries.len());
		resolved.extend(self.static_entries.iter().cloned());
		for (key, expr) in self.dynamic_entries.iter() {
			let value = exec
				.eval(expr)
				.map_err(anyhow::Error::from)
				.and_then(cel_value_to_string)
				.and_then(|value| {
					if value.is_empty() {
						anyhow::bail!("expression produced an empty value");
					}
					validate_value(key, &value)?;
					Ok(value)
				})
				.map_err(|e| {
					AIError::RequestMetadata(strng::format!(
						"{key:?} (expression {:?}): {e}",
						expr.original_expression
					))
				})?;
			resolved.push((key.clone(), value));
		}
		Ok(resolved)
	}

	/// Applies the resolved metadata to a `Converse`/`ConverseStream` body.
	///
	/// Operator entries override caller-supplied keys of the same name. Caller
	/// keys the operator did not claim are kept, up to Bedrock's entry limit;
	/// operator attribution is never the part that gets dropped.
	pub fn apply_to_body(
		&self,
		body: &mut serde_json::Map<String, serde_json::Value>,
		exec: &cel::Executor<'_>,
	) -> Result<(), AIError> {
		let resolved = self.resolve(exec)?;
		self.merge_resolved_into_body(body, resolved);
		Ok(())
	}

	/// Merges already-resolved entries into a `Converse` body; see
	/// [`Self::apply_to_body`] for the precedence rules.
	pub fn merge_resolved_into_body(
		&self,
		body: &mut serde_json::Map<String, serde_json::Value>,
		resolved: Vec<(String, String)>,
	) {
		let mut merged: serde_json::Map<String, serde_json::Value> = resolved
			.into_iter()
			.map(|(k, v)| (k, serde_json::Value::String(v)))
			.collect();
		if let Some(serde_json::Value::Object(existing)) = body.remove(REQUEST_METADATA_FIELD) {
			for (k, v) in existing {
				if merged.len() >= MAX_REQUEST_METADATA_ENTRIES {
					warn!(
						"bedrock request metadata: dropping caller-supplied key {k:?}, entry limit reached"
					);
					continue;
				}
				merged.entry(k).or_insert(v);
			}
		}
		body.insert(
			REQUEST_METADATA_FIELD.to_string(),
			serde_json::Value::Object(merged),
		);
	}

	/// Renders the resolved metadata as the header value for the `InvokeModel`
	/// family of APIs.
	pub fn header_value(&self, exec: &cel::Executor<'_>) -> Result<http::HeaderValue, AIError> {
		let resolved: HashMap<String, String> = self.resolve(exec)?.into_iter().collect();
		let json = serde_json::to_string(&resolved).map_err(AIError::RequestMarshal)?;
		http::HeaderValue::from_str(&json)
			.map_err(|e| AIError::RequestMetadata(strng::format!("header value: {e}")))
	}

	pub(crate) fn static_entries(&self) -> &[(String, String)] {
		&self.static_entries
	}

	pub(crate) fn dynamic_entries(&self) -> &[(String, Arc<cel::Expression>)] {
		&self.dynamic_entries
	}
}

fn validate_key(key: &str) -> anyhow::Result<()> {
	if key.is_empty() {
		anyhow::bail!("request metadata key must not be empty");
	}
	if key.chars().count() > MAX_REQUEST_METADATA_LEN {
		anyhow::bail!("request metadata key {key:?} exceeds {MAX_REQUEST_METADATA_LEN} characters");
	}
	if !ALLOWED_CHARS.is_match(key) {
		anyhow::bail!("request metadata key {key:?} contains characters outside the allowed set");
	}
	Ok(())
}

fn validate_value(key: &str, value: &str) -> anyhow::Result<()> {
	if value.chars().count() > MAX_REQUEST_METADATA_LEN {
		anyhow::bail!(
			"request metadata value for {key:?} exceeds {MAX_REQUEST_METADATA_LEN} characters"
		);
	}
	if !ALLOWED_CHARS.is_match(value) {
		anyhow::bail!("request metadata value for {key:?} contains characters outside the allowed set");
	}
	Ok(())
}

/// Strings, numbers, and booleans (common JWT claim types) stringify; anything
/// else (null, lists, maps) is an error so misattribution fails closed.
fn cel_value_to_string(v: cel::Value) -> anyhow::Result<String> {
	v.always_materialize_owned()
		.as_string()
		.map_err(|e| anyhow::anyhow!("{e}"))
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<BedrockRequestMetadata, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let entries = Vec::<BedrockRequestMetadataEntry>::deserialize(deserializer)?;
	BedrockRequestMetadata::try_new(entries).map_err(serde::de::Error::custom)
}

pub(crate) fn serialize<S>(
	metadata: &BedrockRequestMetadata,
	serializer: S,
) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	let static_entries =
		metadata
			.static_entries()
			.iter()
			.map(|(key, value)| BedrockRequestMetadataEntry {
				key: key.clone(),
				value: Some(value.clone()),
				expression: None,
			});
	let dynamic_entries =
		metadata
			.dynamic_entries()
			.iter()
			.map(|(key, expression)| BedrockRequestMetadataEntry {
				key: key.clone(),
				value: None,
				expression: Some(expression.clone()),
			});
	serializer.collect_seq(static_entries.chain(dynamic_entries))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn entry(
		key: &str,
		value: Option<&str>,
		expression: Option<&str>,
	) -> BedrockRequestMetadataEntry {
		BedrockRequestMetadataEntry {
			key: key.to_string(),
			value: value.map(str::to_string),
			expression: expression
				.map(|e| Arc::new(cel::Expression::new_strict(e).expect("expression should compile"))),
		}
	}

	fn metadata(entries: Vec<BedrockRequestMetadataEntry>) -> BedrockRequestMetadata {
		BedrockRequestMetadata::try_new(entries).expect("metadata should validate")
	}

	fn request_with_headers(headers: &[(&str, &str)]) -> crate::http::Request {
		let mut builder = ::http::Request::builder().uri("http://example.com/v1/chat/completions");
		for (k, v) in headers {
			builder = builder.header(*k, *v);
		}
		builder.body(crate::http::Body::empty()).unwrap()
	}

	#[test]
	fn rejects_more_than_sixteen_entries() {
		let entries = (0..17)
			.map(|i| entry(&format!("k{i}"), Some("v"), None))
			.collect();
		let err = BedrockRequestMetadata::try_new(entries).unwrap_err();
		assert!(err.to_string().contains("at most 16"), "{err}");
	}

	#[test]
	fn rejects_duplicate_keys_and_ambiguous_entries() {
		let err = BedrockRequestMetadata::try_new(vec![
			entry("team", Some("a"), None),
			entry("team", Some("b"), None),
		])
		.unwrap_err();
		assert!(err.to_string().contains("duplicate"), "{err}");

		let err = BedrockRequestMetadata::try_new(vec![entry("team", None, None)]).unwrap_err();
		assert!(err.to_string().contains("exactly one"), "{err}");

		let err =
			BedrockRequestMetadata::try_new(vec![entry("team", Some("a"), Some("jwt.sub"))]).unwrap_err();
		assert!(err.to_string().contains("exactly one"), "{err}");
	}

	#[test]
	fn rejects_invalid_characters_and_lengths_at_load() {
		let err = BedrockRequestMetadata::try_new(vec![entry("te{am}", Some("a"), None)]).unwrap_err();
		assert!(err.to_string().contains("allowed set"), "{err}");

		let err = BedrockRequestMetadata::try_new(vec![entry("team", Some("a\"b"), None)]).unwrap_err();
		assert!(err.to_string().contains("allowed set"), "{err}");

		let long = "x".repeat(MAX_REQUEST_METADATA_LEN + 1);
		let err = BedrockRequestMetadata::try_new(vec![entry("team", Some(&long), None)]).unwrap_err();
		assert!(err.to_string().contains("exceeds"), "{err}");
	}

	#[test]
	fn resolves_static_and_dynamic_entries() {
		let md = metadata(vec![
			entry("CostCenter", Some("12345"), None),
			entry("App", None, Some(r#"request.headers["x-app"]"#)),
		]);
		let req = request_with_headers(&[("x-app", "checkout")]);
		let exec = cel::Executor::new_request(&req);
		let resolved = md.resolve(&exec).unwrap();
		assert_eq!(
			resolved,
			vec![
				("CostCenter".to_string(), "12345".to_string()),
				("App".to_string(), "checkout".to_string()),
			]
		);
	}

	#[test]
	fn fails_closed_when_expression_cannot_resolve() {
		let md = metadata(vec![entry(
			"App",
			None,
			Some(r#"request.headers["x-app"]"#),
		)]);
		let req = request_with_headers(&[]);
		let exec = cel::Executor::new_request(&req);
		let err = md.resolve(&exec).unwrap_err();
		assert!(matches!(err, AIError::RequestMetadata(_)), "{err}");
	}

	#[test]
	fn fails_closed_on_empty_or_invalid_dynamic_value() {
		let md = metadata(vec![entry(
			"App",
			None,
			Some(r#"request.headers["x-app"]"#),
		)]);
		let req = request_with_headers(&[("x-app", "")]);
		let exec = cel::Executor::new_request(&req);
		let err = md.resolve(&exec).unwrap_err();
		assert!(err.to_string().contains("empty value"), "{err}");

		let req = request_with_headers(&[("x-app", "bad\"quote")]);
		let exec = cel::Executor::new_request(&req);
		let err = md.resolve(&exec).unwrap_err();
		assert!(err.to_string().contains("allowed set"), "{err}");
	}

	#[test]
	fn operator_entries_override_caller_supplied_keys_and_keep_the_rest() {
		let md = metadata(vec![
			entry("team", Some("platform"), None),
			entry("user", None, Some(r#"request.headers["x-user"]"#)),
		]);
		let req = request_with_headers(&[("x-user", "alice")]);
		let exec = cel::Executor::new_request(&req);
		let mut body = serde_json::json!({
			"messages": [],
			"requestMetadata": {"team": "caller-says-so", "experiment": "b"}
		});
		md.apply_to_body(body.as_object_mut().unwrap(), &exec)
			.unwrap();
		assert_eq!(
			body["requestMetadata"],
			serde_json::json!({"team": "platform", "user": "alice", "experiment": "b"})
		);
	}

	#[test]
	fn caller_keys_are_dropped_before_operator_keys_at_the_limit() {
		let entries = (0..15)
			.map(|i| entry(&format!("op{i}"), Some("v"), None))
			.collect();
		let md = metadata(entries);
		let req = request_with_headers(&[]);
		let exec = cel::Executor::new_request(&req);
		let mut body = serde_json::json!({
			"requestMetadata": {"c1": "1", "c2": "2", "c3": "3"}
		});
		md.apply_to_body(body.as_object_mut().unwrap(), &exec)
			.unwrap();
		let merged = body["requestMetadata"].as_object().unwrap();
		assert_eq!(merged.len(), MAX_REQUEST_METADATA_ENTRIES);
		assert!((0..15).all(|i| merged.contains_key(&format!("op{i}"))));
	}

	#[test]
	fn header_value_is_json_object() {
		let md = metadata(vec![
			entry("team", Some("platform"), None),
			entry("user", None, Some(r#"request.headers["x-user"]"#)),
		]);
		let req = request_with_headers(&[("x-user", "alice")]);
		let exec = cel::Executor::new_request(&req);
		let value = md.header_value(&exec).unwrap();
		let parsed: HashMap<String, String> = serde_json::from_str(value.to_str().unwrap()).unwrap();
		assert_eq!(parsed["team"], "platform");
		assert_eq!(parsed["user"], "alice");
	}

	#[test]
	fn round_trips_through_serde() {
		let md = metadata(vec![
			entry("team", Some("platform"), None),
			entry("user", None, Some("jwt.sub")),
		]);
		#[derive(serde::Serialize, serde::Deserialize)]
		struct Wrapper {
			#[serde(serialize_with = "serialize", deserialize_with = "deserialize")]
			md: BedrockRequestMetadata,
		}
		let json = serde_json::to_string(&Wrapper { md: md.clone() }).unwrap();
		assert_eq!(
			json,
			r#"{"md":[{"key":"team","value":"platform"},{"key":"user","expression":"jwt.sub"}]}"#
		);
		let back: Wrapper = serde_json::from_str(&json).unwrap();
		assert_eq!(back.md, md);
	}
}
