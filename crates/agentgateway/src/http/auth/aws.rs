use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime};

use aws_config::sts::AssumeRoleProvider;
use aws_config::{BehaviorVersion, SdkConfig};
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials;
use aws_sigv4::http_request::{SignableBody, sign};
use aws_sigv4::sign::v4::SigningParams;
use aws_types::region::Region;
use quick_cache::sync::{Cache as BoundedCache, EntryAction, EntryResult};
use regex::Regex;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::{Mutex, OnceCell};

use crate::llm::bedrock::AwsRegion;
use crate::util::ErrorContext;
use crate::*;

#[derive(Clone, Debug)]
pub struct DefaultAwsServiceName(pub String);

#[apply(schema!)]
#[serde(untagged)]
pub enum AwsAuth {
	/// Use explicit AWS credentials
	#[serde(rename_all = "camelCase")]
	ExplicitConfig {
		#[serde(serialize_with = "ser_redact")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		access_key_id: SecretString,
		#[serde(serialize_with = "ser_redact")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		secret_access_key: SecretString,
		region: Option<String>,
		#[serde(serialize_with = "ser_redact", skip_serializing_if = "Option::is_none")]
		#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
		session_token: Option<SecretString>,
		/// AWS SigV4 signing service name (for example, "bedrock", "bedrock-agentcore", or "execute-api").
		#[serde(skip_serializing_if = "Option::is_none")]
		service_name: Option<String>,
	},
	/// Use implicit AWS authentication (environment variables, IAM roles, etc.)
	#[serde(rename_all = "camelCase")]
	Implicit {
		/// AWS SigV4 signing service name (for example, "bedrock", "bedrock-agentcore", or "execute-api").
		#[serde(skip_serializing_if = "Option::is_none")]
		service_name: Option<String>,
		/// AWS SigV4 signing region (for example, "us-east-1"). If unset, typed AWS
		/// backends may provide this automatically; otherwise the ambient AWS region
		/// is used.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		region: Option<String>,
		/// Optional AWS STS role to assume before signing requests.
		#[serde(skip_serializing_if = "Option::is_none")]
		assume_role: Option<AwsAssumeRole>,
		/// Cached source credentials, populated on first use.
		#[serde(skip)]
		#[cfg_attr(feature = "schema", schemars(skip))]
		source_credentials_cache: AwsCredentialsCache,
		/// Cached AssumeRole credentials, populated on first use.
		#[serde(skip)]
		#[cfg_attr(feature = "schema", schemars(skip))]
		assume_role_cache: AwsAssumeRoleCache,
	},
}

#[derive(Default, Clone)]
pub struct AwsCredentialsCache(Arc<Mutex<Option<Credentials>>>);

impl std::fmt::Debug for AwsCredentialsCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("AwsCredentialsCache")
	}
}

const ASSUME_ROLE_CACHE_CAPACITY: usize = 8192;

/// Bounded cache of assumed-role credentials, keyed by everything that changes the
/// STS AssumeRole call. With dynamic session tags each distinct evaluated tag set is
/// its own entry, so the cache must be bounded, and fetches are single-flight so N
/// concurrent requests for the same key coalesce into one STS call.
#[derive(Clone)]
pub struct AwsAssumeRoleCache(Arc<BoundedCache<AssumeRoleCacheKey, Credentials>>);

impl Default for AwsAssumeRoleCache {
	fn default() -> Self {
		Self(Arc::new(BoundedCache::new(ASSUME_ROLE_CACHE_CAPACITY)))
	}
}

impl AwsAssumeRoleCache {
	async fn get_or_fetch<F, Fut>(
		&self,
		key: AssumeRoleCacheKey,
		fetch: F,
	) -> anyhow::Result<Credentials>
	where
		F: FnOnce() -> Fut,
		Fut: Future<Output = anyhow::Result<Credentials>>,
	{
		let guard = match self
			.0
			.entry_async(&key, |_key, creds| {
				if credentials_valid(creds) {
					EntryAction::Retain(creds.clone())
				} else {
					EntryAction::ReplaceWithGuard
				}
			})
			.await
		{
			EntryResult::Retained(creds) => return Ok(creds),
			EntryResult::Vacant(guard) | EntryResult::Replaced(guard, _) => guard,
			EntryResult::Removed(_, _) | EntryResult::Timeout => unreachable!(),
		};

		let creds = fetch().await?;
		if credentials_valid(&creds) {
			let _ = guard.insert(creds.clone());
		}
		Ok(creds)
	}
}

impl std::fmt::Debug for AwsAssumeRoleCache {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("AwsAssumeRoleCache")
	}
}

#[derive(PartialEq, Eq, Hash)]
#[apply(schema!)]
pub struct AwsAssumeRole {
	/// AWS IAM role ARN to assume.
	pub role_arn: String,
	/// Custom session name (RoleSessionName) for CloudTrail and Cost & Usage Report
	/// attribution. Either a static string or `{expression: ...}` with a CEL
	/// expression evaluated against each request. Max 64 chars, matching
	/// `[\w+=,.@-]`. If unset, the AWS SDK generates a random session name.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub session_name: Option<AwsSessionName>,
	/// Session tags passed to STS AssumeRole for cost attribution. Once activated as
	/// cost allocation tags, each tag surfaces in the AWS Cost & Usage Report under
	/// `resourceTags/user:TagKey`. A tag value is either static (`value`) or a CEL
	/// expression evaluated against each request (`expression`).
	#[serde(
		default,
		skip_serializing_if = "AwsSessionTags::is_empty",
		deserialize_with = "de_session_tags",
		serialize_with = "ser_session_tags"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<AwsSessionTag>"))]
	pub tags: AwsSessionTags,
}

/// An AWS STS session tag passed to AssumeRole for cost attribution.
/// Exactly one of `value` and `expression` must be set.
#[apply(schema!)]
pub struct AwsSessionTag {
	/// Tag key.
	pub key: String,
	/// Static tag value.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub value: Option<String>,
	/// CEL expression evaluated against each request to produce the tag value, for
	/// example `jwt.sub` or `request.headers["x-app"]`. If the expression does not
	/// produce a valid tag value at request time, the request is rejected.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expression: Option<Arc<cel::Expression>>,
}

/// Session name (RoleSessionName) in configuration form: a static string, or a
/// CEL expression evaluated against each request. Untagged, so a plain string
/// keeps its existing meaning.
#[apply(schema!)]
#[serde(
	untagged,
	expecting = "a session name string, or `{expression: ...}` with a valid CEL expression"
)]
pub enum AwsSessionName {
	/// Static session name.
	Static(String),
	/// CEL expression evaluated against each request to produce the session
	/// name, for example `jwt.sub` or `request.headers["x-team"]`. If the
	/// expression does not produce a valid session name at request time, the
	/// request is rejected.
	Dynamic { expression: Arc<cel::Expression> },
}

impl AwsSessionName {
	fn static_name(&self) -> Option<&str> {
		match self {
			Self::Static(name) => Some(name),
			Self::Dynamic { .. } => None,
		}
	}

	fn expression(&self) -> Option<&Arc<cel::Expression>> {
		match self {
			Self::Static(_) => None,
			Self::Dynamic { expression } => Some(expression),
		}
	}

	/// Evaluates a dynamic session name against the request. Fails closed: an
	/// expression that cannot produce a valid session name is an error. Static
	/// names resolve to themselves unvalidated, unchanged from before; STS
	/// rejects an invalid static name loudly on first use.
	fn resolve(&self, req: &http::Request) -> anyhow::Result<String> {
		match self {
			Self::Static(name) => Ok(name.clone()),
			Self::Dynamic { expression } => {
				let exec = cel::Executor::new_request(req);
				exec
					.eval(expression)
					.map_err(anyhow::Error::from)
					.and_then(cel_value_to_string)
					.and_then(|name| {
						validate_session_name(&name)?;
						Ok(name)
					})
					.map_err(|e| {
						anyhow::anyhow!(
							"session name (expression {:?}): {e}",
							expression.original_expression
						)
					})
			},
		}
	}
}

// Dynamic session names cannot derive equality; compare by source expression,
// which is exactly what determines their behavior (same as AwsSessionTags).
impl PartialEq for AwsSessionName {
	fn eq(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::Static(a), Self::Static(b)) => a == b,
			(Self::Dynamic { expression: a }, Self::Dynamic { expression: b }) => {
				a.original_expression == b.original_expression
			},
			_ => false,
		}
	}
}

impl Eq for AwsSessionName {}

impl std::hash::Hash for AwsSessionName {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		match self {
			Self::Static(name) => {
				0u8.hash(state);
				name.hash(state);
			},
			Self::Dynamic { expression } => {
				1u8.hash(state);
				expression.original_expression.hash(state);
			},
		}
	}
}

// STS RoleSessionName limits: https://docs.aws.amazon.com/STS/latest/APIReference/API_AssumeRole.html
const MIN_SESSION_NAME_LEN: usize = 2;
const MAX_SESSION_NAME_LEN: usize = 64;

/// The characters STS accepts in a RoleSessionName.
static SESSION_NAME_CHARSET: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^[\w+=,.@-]*$").expect("static regex compiles"));

fn validate_session_name(name: &str) -> anyhow::Result<()> {
	let len = name.chars().count();
	if !(MIN_SESSION_NAME_LEN..=MAX_SESSION_NAME_LEN).contains(&len) {
		anyhow::bail!(
			"session name must be {MIN_SESSION_NAME_LEN}-{MAX_SESSION_NAME_LEN} characters, got {len}"
		);
	}
	if !SESSION_NAME_CHARSET.is_match(name) {
		anyhow::bail!("session name {name:?} contains characters STS does not accept");
	}
	Ok(())
}

// STS session tag limits: https://docs.aws.amazon.com/STS/latest/APIReference/API_Tag.html
const MAX_SESSION_TAGS: usize = 50;
const MAX_SESSION_TAG_KEY_LEN: usize = 128;
const MAX_SESSION_TAG_VALUE_LEN: usize = 256;

/// The characters STS accepts in session tag keys and values.
static SESSION_TAG_CHARSET: LazyLock<Regex> =
	LazyLock::new(|| Regex::new(r"^[\p{L}\p{Z}\p{N}_.:/=+\-@]*$").expect("static regex compiles"));

/// Session tags in their runtime form: static values pre-resolved, dynamic (CEL)
/// values compiled and evaluated per request.
#[derive(Debug, Clone, Default)]
pub struct AwsSessionTags {
	// Pre-sorted (key, value) pairs so a static-only tag set turns into a cache key
	// with a cheap Arc clone instead of a per-request copy and sort.
	static_tags: Arc<[(String, String)]>,
	// (key, expression) pairs, sorted by key.
	dynamic_tags: Arc<[(String, Arc<cel::Expression>)]>,
}

impl AwsSessionTags {
	/// Validates and splits configured tags into static and dynamic sets. All
	/// STS-side constraints that can be checked without a request are checked here,
	/// so config errors surface at load time rather than per request.
	pub fn try_new(tags: Vec<AwsSessionTag>) -> anyhow::Result<Self> {
		if tags.len() > MAX_SESSION_TAGS {
			anyhow::bail!(
				"at most {MAX_SESSION_TAGS} session tags are allowed, got {}",
				tags.len()
			);
		}
		let mut keys = HashSet::with_capacity(tags.len());
		let mut static_tags = Vec::new();
		let mut dynamic_tags = Vec::new();
		for tag in tags {
			validate_session_tag_key(&tag.key)?;
			if !keys.insert(tag.key.clone()) {
				anyhow::bail!("duplicate session tag key {:?}", tag.key);
			}
			match (tag.value, tag.expression) {
				(Some(value), None) => {
					validate_session_tag_value(&tag.key, &value)?;
					static_tags.push((tag.key, value));
				},
				(None, Some(expression)) => dynamic_tags.push((tag.key, expression)),
				_ => anyhow::bail!(
					"session tag {:?} must set exactly one of 'value' or 'expression'",
					tag.key
				),
			}
		}
		static_tags.sort();
		dynamic_tags.sort_by(|(a, _), (b, _)| a.cmp(b));
		Ok(Self {
			static_tags: static_tags.into(),
			dynamic_tags: dynamic_tags.into(),
		})
	}

	/// Builds a static-only tag set from (key, value) pairs, sorted into the
	/// canonical form so equality is independent of configuration order.
	pub fn from_static<I: IntoIterator<Item = (String, String)>>(tags: I) -> Self {
		let mut tags: Vec<(String, String)> = tags.into_iter().collect();
		tags.sort();
		Self {
			static_tags: tags.into(),
			dynamic_tags: Default::default(),
		}
	}

	pub fn is_empty(&self) -> bool {
		self.static_tags.is_empty() && self.dynamic_tags.is_empty()
	}

	pub fn has_dynamic(&self) -> bool {
		!self.dynamic_tags.is_empty()
	}

	pub fn static_tags(&self) -> Arc<[(String, String)]> {
		self.static_tags.clone()
	}

	pub fn expressions(&self) -> impl Iterator<Item = &cel::Expression> {
		self.dynamic_tags.iter().map(|(_, e)| e.as_ref())
	}

	/// Evaluates dynamic tags against the request and merges them with the static
	/// tags into the sorted form used for both the cache key and the STS call.
	/// Fails closed: an expression that cannot produce a valid tag value is an error.
	fn resolve(&self, req: &http::Request) -> anyhow::Result<Arc<[(String, String)]>> {
		let exec = cel::Executor::new_request(req);
		let mut resolved: Vec<(String, String)> =
			Vec::with_capacity(self.static_tags.len() + self.dynamic_tags.len());
		resolved.extend(self.static_tags.iter().cloned());
		for (key, expr) in self.dynamic_tags.iter() {
			let value = exec
				.eval(expr)
				.map_err(anyhow::Error::from)
				.and_then(cel_value_to_string)
				.and_then(|value| {
					if value.is_empty() {
						anyhow::bail!("expression produced an empty value");
					}
					validate_session_tag_value(key, &value)?;
					Ok(value)
				})
				.map_err(|e| {
					anyhow::anyhow!(
						"session tag {:?} (expression {:?}): {e}",
						key,
						expr.original_expression
					)
				})?;
			resolved.push((key.clone(), value));
		}
		resolved.sort();
		Ok(resolved.into())
	}
}

// Dynamic tags cannot derive equality; compare them by (key, source expression),
// which is exactly what determines their behavior.
impl PartialEq for AwsSessionTags {
	fn eq(&self, other: &Self) -> bool {
		self.static_tags == other.static_tags
			&& self.dynamic_tags.len() == other.dynamic_tags.len()
			&& self
				.dynamic_tags
				.iter()
				.zip(other.dynamic_tags.iter())
				.all(|((ak, ae), (bk, be))| ak == bk && ae.original_expression == be.original_expression)
	}
}

impl Eq for AwsSessionTags {}

impl std::hash::Hash for AwsSessionTags {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.static_tags.hash(state);
		for (key, expr) in self.dynamic_tags.iter() {
			key.hash(state);
			expr.original_expression.hash(state);
		}
	}
}

fn validate_session_tag_key(key: &str) -> anyhow::Result<()> {
	if key.is_empty() || key.chars().count() > MAX_SESSION_TAG_KEY_LEN {
		anyhow::bail!("session tag key {key:?} must be 1-{MAX_SESSION_TAG_KEY_LEN} characters");
	}
	if !SESSION_TAG_CHARSET.is_match(key) {
		anyhow::bail!("session tag key {key:?} contains characters STS does not accept");
	}
	Ok(())
}

fn validate_session_tag_value(key: &str, value: &str) -> anyhow::Result<()> {
	if value.chars().count() > MAX_SESSION_TAG_VALUE_LEN {
		anyhow::bail!("session tag {key:?} value exceeds {MAX_SESSION_TAG_VALUE_LEN} characters");
	}
	if !SESSION_TAG_CHARSET.is_match(value) {
		anyhow::bail!("session tag {key:?} value contains characters STS does not accept");
	}
	Ok(())
}

/// Coerces a CEL evaluation result into a tag value or session name. Strings,
/// numbers, and booleans (common JWT claim types) stringify via
/// [`cel::Value::as_string`]; anything else (null, lists, maps) is an error so
/// misattribution fails closed.
fn cel_value_to_string(v: cel::Value) -> anyhow::Result<String> {
	// Materialize Dynamic so nested lookups (e.g. JWT claims) are concrete values.
	v.always_materialize_owned()
		.as_string()
		.map_err(|e| anyhow::anyhow!("{e}"))
}

fn de_session_tags<'de, D>(deserializer: D) -> Result<AwsSessionTags, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let tags = Vec::<AwsSessionTag>::deserialize(deserializer)?;
	AwsSessionTags::try_new(tags).map_err(serde::de::Error::custom)
}

fn ser_session_tags<S>(tags: &AwsSessionTags, serializer: S) -> Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	let static_entries = tags.static_tags.iter().map(|(key, value)| AwsSessionTag {
		key: key.clone(),
		value: Some(value.clone()),
		expression: None,
	});
	let dynamic_entries = tags
		.dynamic_tags
		.iter()
		.map(|(key, expression)| AwsSessionTag {
			key: key.clone(),
			value: None,
			expression: Some(expression.clone()),
		});
	serializer.collect_seq(static_entries.chain(dynamic_entries))
}

impl AwsAuth {
	fn service_name(&self) -> Option<&str> {
		match self {
			AwsAuth::ExplicitConfig { service_name, .. } | AwsAuth::Implicit { service_name, .. } => {
				service_name.as_deref()
			},
		}
	}

	fn assume_role(&self) -> Option<&AwsAssumeRole> {
		match self {
			AwsAuth::ExplicitConfig { .. } => None,
			AwsAuth::Implicit { assume_role, .. } => assume_role.as_ref(),
		}
	}

	fn assume_role_cache(&self) -> Option<&AwsAssumeRoleCache> {
		match self {
			AwsAuth::ExplicitConfig { .. } => None,
			AwsAuth::Implicit {
				assume_role_cache, ..
			} => Some(assume_role_cache),
		}
	}

	/// CEL expressions this auth config evaluates per request (dynamic session
	/// tags and session name), for registration with the CEL context builder.
	pub fn cel_expressions(&self) -> impl Iterator<Item = &cel::Expression> {
		self.assume_role().into_iter().flat_map(|assume_role| {
			assume_role.tags.expressions().chain(
				assume_role
					.session_name
					.as_ref()
					.and_then(|name| name.expression())
					.map(|e| e.as_ref()),
			)
		})
	}

	fn source_credentials_cache(&self) -> Option<&AwsCredentialsCache> {
		match self {
			AwsAuth::ExplicitConfig { .. } => None,
			AwsAuth::Implicit {
				source_credentials_cache,
				..
			} => Some(source_credentials_cache),
		}
	}
}

fn signing_service_name<'a>(req: &'a http::Request, aws_auth: &'a AwsAuth) -> &'a str {
	aws_auth
		.service_name()
		.or_else(|| {
			req
				.extensions()
				.get::<DefaultAwsServiceName>()
				.map(|default| default.0.as_str())
		})
		.unwrap_or("bedrock")
}

pub(super) async fn sign_request(
	req: &mut http::Request,
	aws_auth: &AwsAuth,
) -> anyhow::Result<()> {
	// Resolve any dynamic (CEL) session tags and session name first, while the
	// request is intact. The CEL context reads headers and extensions (JWT
	// claims, etc.), which the proxy keeps on the request until after late
	// backend auth. Fails closed: an expression that cannot produce a valid
	// value rejects the request.
	let resolved_tags = match aws_auth.assume_role() {
		Some(assume_role) if assume_role.tags.has_dynamic() => Some(assume_role.tags.resolve(req)?),
		_ => None,
	};
	let resolved_session_name = match aws_auth.assume_role().and_then(|a| a.session_name.as_ref()) {
		Some(name @ AwsSessionName::Dynamic { .. }) => Some(name.resolve(req)?),
		_ => None,
	};
	let lim = crate::http::buffer_limit(req);
	let orig_body = std::mem::take(req.body_mut());
	// Get the region based on auth mode
	let region = match aws_auth {
		AwsAuth::ExplicitConfig {
			region: Some(region),
			..
		}
		| AwsAuth::Implicit {
			region: Some(region),
			..
		} => region.as_str(),
		AwsAuth::ExplicitConfig { region: None, .. } | AwsAuth::Implicit { region: None, .. } => {
			// Try to get region from request extensions first, then fall back to AWS config
			if let Some(aws_region) = req.extensions().get::<AwsRegion>() {
				aws_region.region.as_str()
			} else {
				// Fall back to region from AWS config
				let config = Box::pin(sdk_config()).await;
				config.region().map(|r| r.as_ref()).ok_or(anyhow::anyhow!(
					"No region found in AWS config or request extensions"
				))?
			}
		},
	};
	let creds = tokio::time::timeout(
		super::CLOUD_AUTH_TIMEOUT,
		Box::pin(load_credentials(
			aws_auth,
			region,
			resolved_tags,
			resolved_session_name,
		)),
	)
	.await
	.ctx("AWS credential fetch timed out after 5s")??
	.into();

	let service = signing_service_name(req, aws_auth);
	trace!("AWS signing with region: {}, service: {}", region, service);

	// Sign the request
	let signing_params = SigningParams::builder()
		.identity(&creds)
		.region(region)
		.name(service)
		.time(std::time::SystemTime::now())
		.settings(aws_sigv4::http_request::SigningSettings::default())
		.build()?
		.into();

	let body = http::read_body_with_limit(orig_body, lim).await?;
	let signable_request = aws_sigv4::http_request::SignableRequest::new(
		req.method().as_str(),
		req.uri().to_string().replace("http://", "https://"),
		req
			.headers()
			.iter()
			.filter_map(|(k, v)| {
				std::str::from_utf8(v.as_bytes())
					.ok()
					.map(|v_str| (k.as_str(), v_str))
			})
			.filter(|(k, _)| should_sign_header(k)),
		// SignableBody::UnsignedPayload,
		SignableBody::Bytes(body.as_ref()),
	)?;

	let (signature, _sig) = sign(signable_request, &signing_params)?.into_parts();
	signature.apply_to_request_http1x(req);

	req.headers_mut().insert(
		http::header::CONTENT_LENGTH,
		http::HeaderValue::from_str(&format!("{}", body.as_ref().len()))?,
	);
	*req.body_mut() = http::Body::from(body);

	trace!("signed AWS request");
	Ok(())
}

fn should_sign_header(name: &str) -> bool {
	name == http::header::HOST.as_str()
		|| name == http::header::CONTENT_TYPE.as_str()
		|| name == http::header::DATE.as_str()
		|| name.starts_with("x-amz-")
		|| name.starts_with("x-amzn-")
}

static SDK_CONFIG: OnceCell<SdkConfig> = OnceCell::const_new();
async fn sdk_config<'a>() -> &'a SdkConfig {
	SDK_CONFIG
		.get_or_init(|| async { aws_config::load_defaults(BehaviorVersion::v2026_01_12()).await })
		.await
}

async fn load_credentials(
	aws_auth: &AwsAuth,
	signing_region: &str,
	resolved_tags: Option<Arc<[(String, String)]>>,
	resolved_session_name: Option<String>,
) -> anyhow::Result<Credentials> {
	if let (Some(assume_role), Some(cache)) = (aws_auth.assume_role(), aws_auth.assume_role_cache()) {
		load_assumed_credentials(
			assume_role,
			cache,
			signing_region,
			resolved_tags,
			resolved_session_name,
		)
		.await
	} else {
		load_source_credentials(aws_auth).await
	}
}

async fn load_source_credentials(aws_auth: &AwsAuth) -> anyhow::Result<Credentials> {
	match aws_auth {
		AwsAuth::ExplicitConfig {
			access_key_id,
			secret_access_key,
			session_token,
			region: _,
			service_name: _,
		} => {
			// Use explicit credentials
			let mut builder = Credentials::builder()
				.access_key_id(access_key_id.expose_secret())
				.secret_access_key(secret_access_key.expose_secret())
				.provider_name("bedrock");

			if let Some(token) = session_token {
				builder = builder.session_token(token.expose_secret());
			}

			Ok(builder.build())
		},
		AwsAuth::Implicit { .. } => {
			let cache = aws_auth
				.source_credentials_cache()
				.expect("implicit AWS auth always has a source credential cache");
			{
				let mut cached = cache.0.lock().await;
				if let Some(creds) = cached.as_ref() {
					if credentials_valid(creds) {
						return Ok(creds.clone());
					}
					*cached = None;
				}
			}

			// Load AWS configuration and credentials from environment/IAM
			let config = Box::pin(sdk_config()).await;

			// Get credentials from the config
			let creds = config
				.credentials_provider()
				.ok_or(anyhow::anyhow!(
					"No credentials provider found in AWS config"
				))?
				.provide_credentials()
				.await?;
			*cache.0.lock().await = Some(creds.clone());
			Ok(creds)
		},
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AssumeRoleCacheKey {
	role_arn: String,
	resolved_sts_region: String,
	session_name: Option<String>,
	/// Sorted (key, value) pairs so the cache key is stable regardless of tag order.
	tags: Arc<[(String, String)]>,
}

/// The session name for the STS call: the per-request resolved name when the
/// configured name is dynamic, otherwise the configured static name.
fn effective_session_name(
	assume_role: &AwsAssumeRole,
	resolved_session_name: Option<String>,
) -> Option<String> {
	resolved_session_name.or_else(|| {
		assume_role
			.session_name
			.as_ref()
			.and_then(|name| name.static_name().map(String::from))
	})
}

const ASSUMED_CREDENTIAL_REFRESH_BUFFER: Duration = Duration::from_secs(60);

async fn load_assumed_credentials(
	assume_role: &AwsAssumeRole,
	cache: &AwsAssumeRoleCache,
	signing_region: &str,
	resolved_tags: Option<Arc<[(String, String)]>>,
	resolved_session_name: Option<String>,
) -> anyhow::Result<Credentials> {
	let sts_region = resolve_sts_region(assume_role, signing_region).await?;
	// resolved_tags and resolved_session_name are Some iff the corresponding
	// dynamic config is set (see sign_request); static-only configs use the
	// configured values directly.
	let tags = resolved_tags.unwrap_or_else(|| assume_role.tags.static_tags());
	let session_name = effective_session_name(assume_role, resolved_session_name);
	let key = AssumeRoleCacheKey {
		role_arn: assume_role.role_arn.clone(),
		resolved_sts_region: sts_region.clone(),
		session_name,
		tags,
	};

	cache
		.get_or_fetch(key.clone(), || async move {
			let config = Box::pin(sdk_config()).await;
			let mut builder = AssumeRoleProvider::builder(&assume_role.role_arn)
				.configure(config)
				.region(Region::new(sts_region));

			if let Some(session_name) = &key.session_name {
				builder = builder.session_name(session_name.clone());
			}

			if !key.tags.is_empty() {
				builder = builder.tags(key.tags.iter().cloned());
			}

			let source_credentials_provider = config.credentials_provider().ok_or(anyhow::anyhow!(
				"No credentials provider found in AWS config"
			))?;
			let provider = builder
				.build_from_provider(source_credentials_provider.clone())
				.await;
			Ok(provider.provide_credentials().await?)
		})
		.await
}

async fn resolve_sts_region(
	_assume_role: &AwsAssumeRole,
	signing_region: &str,
) -> anyhow::Result<String> {
	if !signing_region.is_empty() {
		return Ok(signing_region.to_string());
	}
	let config = Box::pin(sdk_config()).await;
	config
		.region()
		.map(|r| r.as_ref().to_string())
		.ok_or(anyhow::anyhow!(
			"No region found in AWS config or request extensions"
		))
}

fn credentials_valid(creds: &Credentials) -> bool {
	match creds.expiry() {
		Some(expiry) => expiry
			.duration_since(SystemTime::now())
			.is_ok_and(|ttl| ttl > ASSUMED_CREDENTIAL_REFRESH_BUFFER),
		None => true,
	}
}

#[cfg(test)]
mod cache_key_tests {
	use super::*;

	fn tags(pairs: &[(&str, &str)]) -> AwsSessionTags {
		AwsSessionTags::from_static(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())))
	}

	fn key_for(assume_role: &AwsAssumeRole, region: &str) -> AssumeRoleCacheKey {
		AssumeRoleCacheKey {
			role_arn: assume_role.role_arn.clone(),
			resolved_sts_region: region.to_string(),
			session_name: assume_role
				.session_name
				.as_ref()
				.and_then(|name| name.static_name().map(String::from)),
			tags: assume_role.tags.static_tags(),
		}
	}

	#[test]
	fn different_session_names_produce_different_keys() {
		let base = AwsAssumeRole {
			role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
			session_name: Some(AwsSessionName::Static("team-a".to_string())),
			tags: tags(&[]),
		};
		let other = AwsAssumeRole {
			session_name: Some(AwsSessionName::Static("team-b".to_string())),
			..base.clone()
		};
		assert_ne!(key_for(&base, "us-east-1"), key_for(&other, "us-east-1"));
	}

	#[test]
	fn effective_session_name_prefers_resolved_over_static() {
		let static_config = AwsAssumeRole {
			role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
			session_name: Some(AwsSessionName::Static("static-name".to_string())),
			tags: tags(&[]),
		};
		// Static config, nothing resolved: the configured name is used.
		assert_eq!(
			effective_session_name(&static_config, None),
			Some("static-name".to_string())
		);
		// A resolved name always wins; it only exists when the config is dynamic.
		assert_eq!(
			effective_session_name(&static_config, Some("resolved-name".to_string())),
			Some("resolved-name".to_string())
		);
		let unset = AwsAssumeRole {
			session_name: None,
			..static_config
		};
		assert_eq!(effective_session_name(&unset, None), None);
	}

	#[test]
	fn dynamic_session_name_resolving_to_static_value_produces_equal_key() {
		// A dynamic session name that evaluates to X keys the cache identically to
		// a static session name X: only the resolved value matters.
		let static_key = AssumeRoleCacheKey {
			role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
			resolved_sts_region: "us-east-1".to_string(),
			session_name: Some("team-a-invoicer".to_string()),
			tags: tags(&[]).static_tags(),
		};
		let resolved_key = AssumeRoleCacheKey {
			session_name: Some("team-a-invoicer".to_string()),
			..static_key.clone()
		};
		assert_eq!(static_key, resolved_key);
	}

	#[test]
	fn different_tags_produce_different_keys() {
		let base = AwsAssumeRole {
			role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
			session_name: None,
			tags: tags(&[("Team", "acme-payments")]),
		};
		let other = AwsAssumeRole {
			tags: tags(&[("Team", "acme-billing")]),
			..base.clone()
		};
		assert_ne!(key_for(&base, "us-east-1"), key_for(&other, "us-east-1"));
	}

	#[test]
	fn dynamic_tags_resolving_to_static_values_produce_equal_keys() {
		// A dynamic tag that evaluates to X keys the cache identically to a static
		// tag configured as X: only resolved values matter, not how they were produced.
		let static_key = AssumeRoleCacheKey {
			role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
			resolved_sts_region: "us-east-1".to_string(),
			session_name: None,
			tags: tags(&[("Team", "acme")]).static_tags(),
		};
		let resolved_key = AssumeRoleCacheKey {
			tags: Arc::from([("Team".to_string(), "acme".to_string())]),
			..static_key.clone()
		};
		assert_eq!(static_key, resolved_key);
	}

	#[test]
	fn tag_order_does_not_affect_key() {
		// Tags are canonicalized (sorted) at deserialization, so configs that only
		// differ in tag order deserialize to equal values and thus equal cache keys.
		let a: AwsAssumeRole = serde_json::from_value(serde_json::json!({
			"roleArn": "arn:aws:iam::123456789012:role/backend",
			"tags": [
				{"key": "Team", "value": "acme"},
				{"key": "App", "value": "invoicer"},
			],
		}))
		.expect("assume role should deserialize");
		let b: AwsAssumeRole = serde_json::from_value(serde_json::json!({
			"roleArn": "arn:aws:iam::123456789012:role/backend",
			"tags": [
				{"key": "App", "value": "invoicer"},
				{"key": "Team", "value": "acme"},
			],
		}))
		.expect("assume role should deserialize");
		assert_eq!(a.tags, b.tags);
		assert_eq!(key_for(&a, "us-east-1"), key_for(&b, "us-east-1"));
	}
}

#[cfg(test)]
mod resolve_tags_tests {
	use secrecy::SecretString;

	use super::*;
	use crate::http::jwt::Claims;

	fn tag(key: &str, value: Option<&str>, expression: Option<&str>) -> AwsSessionTag {
		AwsSessionTag {
			key: key.to_string(),
			value: value.map(str::to_string),
			expression: expression
				.map(|e| Arc::new(cel::Expression::new_strict(e).expect("expression should compile"))),
		}
	}

	fn request(headers: &[(&str, &str)], claims: Option<serde_json::Value>) -> http::Request {
		let mut builder = ::http::Request::builder().uri("http://example.com/");
		for (k, v) in headers {
			builder = builder.header(*k, *v);
		}
		let mut req = builder.body(crate::http::Body::empty()).expect("request");
		if let Some(serde_json::Value::Object(claims)) = claims {
			req.extensions_mut().insert(Claims {
				inner: claims,
				jwt: SecretString::new("header.payload.signature".into()),
			});
		}
		req
	}

	fn session_tags(tags: Vec<AwsSessionTag>) -> AwsSessionTags {
		AwsSessionTags::try_new(tags).expect("tags should validate")
	}

	#[test]
	fn resolves_header_and_jwt_tags_merged_with_static() {
		let tags = session_tags(vec![
			tag("Team", Some("acme"), None),
			tag("App", None, Some(r#"request.headers["x-app"]"#)),
			tag("User", None, Some("jwt.sub")),
		]);
		let req = request(
			&[("x-app", "invoicer")],
			Some(serde_json::json!({"sub": "user@example.com"})),
		);
		let resolved = tags.resolve(&req).expect("tags should resolve");
		assert_eq!(
			resolved.as_ref(),
			&[
				("App".to_string(), "invoicer".to_string()),
				("Team".to_string(), "acme".to_string()),
				("User".to_string(), "user@example.com".to_string()),
			]
		);
	}

	#[test]
	fn stringifies_numeric_claims() {
		let tags = session_tags(vec![tag("OrgId", None, Some("jwt.org_id"))]);
		let req = request(&[], Some(serde_json::json!({"org_id": 42})));
		let resolved = tags.resolve(&req).expect("tags should resolve");
		assert_eq!(
			resolved.as_ref(),
			&[("OrgId".to_string(), "42".to_string())]
		);
	}

	#[test]
	fn missing_header_fails_closed() {
		let tags = session_tags(vec![tag("App", None, Some(r#"request.headers["x-app"]"#))]);
		let err = tags.resolve(&request(&[], None)).expect_err("should fail");
		assert!(
			err.to_string().contains("App"),
			"error names the tag: {err}"
		);
	}

	#[test]
	fn empty_value_fails_closed() {
		let tags = session_tags(vec![tag("App", None, Some(r#"request.headers["x-app"]"#))]);
		let err = tags
			.resolve(&request(&[("x-app", "")], None))
			.expect_err("should fail");
		assert!(err.to_string().contains("empty"), "got: {err}");
	}

	#[test]
	fn invalid_charset_fails_closed() {
		let tags = session_tags(vec![tag("App", None, Some(r#"request.headers["x-app"]"#))]);
		let err = tags
			.resolve(&request(&[("x-app", "a,b")], None))
			.expect_err("should fail");
		assert!(
			err.to_string().contains("STS does not accept"),
			"got: {err}"
		);
	}

	#[test]
	fn oversized_value_fails_closed() {
		let tags = session_tags(vec![tag("App", None, Some(r#"request.headers["x-app"]"#))]);
		let long = "a".repeat(MAX_SESSION_TAG_VALUE_LEN + 1);
		let err = tags
			.resolve(&request(&[("x-app", long.as_str())], None))
			.expect_err("should fail");
		assert!(err.to_string().contains("exceeds"), "got: {err}");
	}

	#[tokio::test]
	async fn sign_request_fails_closed_when_dynamic_tag_cannot_resolve() {
		let auth = AwsAuth::Implicit {
			service_name: None,
			region: None,
			assume_role: Some(AwsAssumeRole {
				role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
				session_name: None,
				tags: session_tags(vec![tag("App", None, Some(r#"request.headers["x-app"]"#))]),
			}),
			source_credentials_cache: Default::default(),
			assume_role_cache: Default::default(),
		};
		// No x-app header: resolution fails before any credential loading or STS call.
		let mut req = request(&[], None);
		let err = sign_request(&mut req, &auth)
			.await
			.expect_err("must reject the request rather than sign it unattributed");
		assert!(
			err.to_string().contains("App"),
			"error names the tag: {err}"
		);
	}

	#[tokio::test]
	async fn sign_request_fails_closed_when_expression_did_not_compile() {
		// xds compiles expressions permissively: a bad expression becomes one that
		// always fails at evaluation, so only requests hitting this tag fail.
		let (expression, err) = cel::Expression::new_permissive("this is not cel (");
		assert!(err.is_some(), "expression should fail to compile");
		let auth = AwsAuth::Implicit {
			service_name: None,
			region: None,
			assume_role: Some(AwsAssumeRole {
				role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
				session_name: None,
				tags: AwsSessionTags::try_new(vec![AwsSessionTag {
					key: "App".to_string(),
					value: None,
					expression: Some(Arc::new(expression)),
				}])
				.expect("permissive expression should pass config validation"),
			}),
			source_credentials_cache: Default::default(),
			assume_role_cache: Default::default(),
		};
		let mut req = request(&[("x-app", "invoicer")], None);
		let err = sign_request(&mut req, &auth)
			.await
			.expect_err("uncompilable expression must fail the request");
		assert!(
			err.to_string().contains("App"),
			"error names the tag: {err}"
		);
	}
}

#[cfg(test)]
mod resolve_session_name_tests {
	use secrecy::SecretString;

	use super::*;
	use crate::http::jwt::Claims;

	fn dynamic(expression: &str) -> AwsSessionName {
		AwsSessionName::Dynamic {
			expression: Arc::new(
				cel::Expression::new_strict(expression).expect("expression should compile"),
			),
		}
	}

	fn request(headers: &[(&str, &str)], claims: Option<serde_json::Value>) -> http::Request {
		let mut builder = ::http::Request::builder().uri("http://example.com/");
		for (k, v) in headers {
			builder = builder.header(*k, *v);
		}
		let mut req = builder.body(crate::http::Body::empty()).expect("request");
		if let Some(serde_json::Value::Object(claims)) = claims {
			req.extensions_mut().insert(Claims {
				inner: claims,
				jwt: SecretString::new("header.payload.signature".into()),
			});
		}
		req
	}

	#[test]
	fn resolves_from_header_and_jwt() {
		let name = dynamic(r#"request.headers["x-team"] + "-" + jwt.sub"#);
		let req = request(
			&[("x-team", "acme")],
			Some(serde_json::json!({"sub": "user@example.com"})),
		);
		assert_eq!(
			name.resolve(&req).expect("should resolve"),
			"acme-user@example.com"
		);
	}

	#[test]
	fn stringifies_numeric_claim() {
		let name = dynamic("jwt.org_id");
		let req = request(&[], Some(serde_json::json!({"org_id": 42})));
		assert_eq!(name.resolve(&req).expect("should resolve"), "42");
	}

	#[test]
	fn static_name_resolves_to_itself() {
		let name = AwsSessionName::Static("team-a".to_string());
		assert_eq!(
			name.resolve(&request(&[], None)).expect("resolves"),
			"team-a"
		);
	}

	#[test]
	fn missing_header_fails_closed() {
		let name = dynamic(r#"request.headers["x-team"]"#);
		let err = name.resolve(&request(&[], None)).expect_err("should fail");
		assert!(
			err.to_string().contains("session name"),
			"error names the session name: {err}"
		);
	}

	#[test]
	fn too_short_fails_closed() {
		let name = dynamic(r#"request.headers["x-team"]"#);
		let err = name
			.resolve(&request(&[("x-team", "a")], None))
			.expect_err("should fail");
		assert!(err.to_string().contains("2-64"), "got: {err}");
	}

	#[test]
	fn too_long_fails_closed() {
		let name = dynamic(r#"request.headers["x-team"]"#);
		let long = "a".repeat(MAX_SESSION_NAME_LEN + 1);
		let err = name
			.resolve(&request(&[("x-team", long.as_str())], None))
			.expect_err("should fail");
		assert!(err.to_string().contains("2-64"), "got: {err}");
	}

	#[test]
	fn invalid_charset_fails_closed() {
		// RoleSessionName is stricter than session tag values: spaces and colons
		// are accepted in tags but not in the session name.
		let name = dynamic(r#"request.headers["x-team"]"#);
		let err = name
			.resolve(&request(&[("x-team", "team a")], None))
			.expect_err("should fail");
		assert!(
			err.to_string().contains("STS does not accept"),
			"got: {err}"
		);
	}

	#[tokio::test]
	async fn sign_request_fails_closed_when_session_name_cannot_resolve() {
		let auth = AwsAuth::Implicit {
			service_name: None,
			region: None,
			assume_role: Some(AwsAssumeRole {
				role_arn: "arn:aws:iam::123456789012:role/backend".to_string(),
				session_name: Some(dynamic(r#"request.headers["x-team"]"#)),
				tags: Default::default(),
			}),
			source_credentials_cache: Default::default(),
			assume_role_cache: Default::default(),
		};
		// No x-team header: resolution fails before any credential loading or STS call.
		let mut req = request(&[], None);
		let err = sign_request(&mut req, &auth)
			.await
			.expect_err("must reject the request rather than sign it misattributed");
		assert!(
			err.to_string().contains("session name"),
			"error names the session name: {err}"
		);
	}
}

#[cfg(test)]
mod assume_role_cache_tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::*;

	fn creds(expires_in: Option<Duration>) -> Credentials {
		Credentials::new(
			"AKID",
			"SECRET",
			None,
			expires_in.map(|ttl| SystemTime::now() + ttl),
			"test",
		)
	}

	fn key(role: &str, tags: &[(&str, &str)]) -> AssumeRoleCacheKey {
		AssumeRoleCacheKey {
			role_arn: format!("arn:aws:iam::123456789012:role/{role}"),
			resolved_sts_region: "us-east-1".to_string(),
			session_name: None,
			tags: tags
				.iter()
				.map(|(k, v)| (k.to_string(), v.to_string()))
				.collect(),
		}
	}

	async fn fetch(
		cache: &AwsAssumeRoleCache,
		key: AssumeRoleCacheKey,
		calls: &AtomicUsize,
		expires_in: Option<Duration>,
	) -> anyhow::Result<Credentials> {
		cache
			.get_or_fetch(key, || {
				calls.fetch_add(1, Ordering::Relaxed);
				std::future::ready(Ok(creds(expires_in)))
			})
			.await
	}

	#[tokio::test]
	async fn same_key_fetches_once() {
		let cache = AwsAssumeRoleCache::default();
		let calls = AtomicUsize::new(0);
		let k = key("backend", &[("User", "alice")]);
		fetch(&cache, k.clone(), &calls, None).await.unwrap();
		fetch(&cache, k, &calls, None).await.unwrap();
		assert_eq!(calls.load(Ordering::Relaxed), 1);
	}

	#[tokio::test]
	async fn distinct_tag_values_fetch_separately() {
		let cache = AwsAssumeRoleCache::default();
		let calls = AtomicUsize::new(0);
		fetch(&cache, key("backend", &[("User", "alice")]), &calls, None)
			.await
			.unwrap();
		fetch(&cache, key("backend", &[("User", "bob")]), &calls, None)
			.await
			.unwrap();
		assert_eq!(calls.load(Ordering::Relaxed), 2);
	}

	#[tokio::test]
	async fn concurrent_requests_for_same_key_are_single_flight() {
		let cache = AwsAssumeRoleCache::default();
		let calls = AtomicUsize::new(0);
		let k = key("backend", &[("User", "alice")]);
		let slow_fetch = || async {
			let n = calls.fetch_add(1, Ordering::Relaxed);
			tokio::time::sleep(Duration::from_millis(50)).await;
			assert_eq!(n, 0, "only one concurrent fetch should run");
			Ok(creds(None))
		};
		let (a, b) = tokio::join!(
			cache.get_or_fetch(k.clone(), slow_fetch),
			cache.get_or_fetch(k, slow_fetch),
		);
		a.unwrap();
		b.unwrap();
		assert_eq!(calls.load(Ordering::Relaxed), 1);
	}

	#[tokio::test]
	async fn near_expiry_credentials_are_not_cached() {
		let cache = AwsAssumeRoleCache::default();
		let calls = AtomicUsize::new(0);
		let k = key("backend", &[]);
		// Within the refresh buffer: usable for this request, but not worth caching.
		let ttl = Some(ASSUMED_CREDENTIAL_REFRESH_BUFFER / 2);
		fetch(&cache, k.clone(), &calls, ttl).await.unwrap();
		fetch(&cache, k, &calls, ttl).await.unwrap();
		assert_eq!(calls.load(Ordering::Relaxed), 2);
	}

	#[tokio::test]
	async fn fetch_errors_are_not_cached() {
		let cache = AwsAssumeRoleCache::default();
		let calls = AtomicUsize::new(0);
		let k = key("backend", &[]);
		let err = cache
			.get_or_fetch(k.clone(), || {
				calls.fetch_add(1, Ordering::Relaxed);
				std::future::ready(Err(anyhow::anyhow!("sts unavailable")))
			})
			.await;
		assert!(err.is_err());
		fetch(&cache, k, &calls, None).await.unwrap();
		assert_eq!(calls.load(Ordering::Relaxed), 2);
	}
}
