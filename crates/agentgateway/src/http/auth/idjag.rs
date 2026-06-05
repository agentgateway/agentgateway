//! OAuth Identity Assertion Authorization Grant (ID-JAG / Cross App Access).
//!
//! Implements the *client* side of `draft-ietf-oauth-identity-assertion-authz-grant`.
//! On a backend call, the validated inbound user identity (a JWT placed in the request
//! extensions by the `jwtAuthentication` policy) is turned into a backend-scoped access
//! token via two token-endpoint calls:
//!
//! 1. RFC 8693 token exchange against the user's IdP authorization server, yielding an
//!    ID-JAG assertion (`requested_token_type=...:id-jag`).
//! 2. RFC 7523 JWT-bearer grant presenting that ID-JAG to the resource's authorization
//!    server, yielding a Bearer access token.
//!
//! The Bearer token is attached to the outbound backend request and cached per
//! `(subject, audience, scope, resource)` until shortly before it expires.
//!
//! Out of scope for now (see issue #2029): DPoP sender-constraint (RFC 9449), `.well-known`
//! endpoint discovery (RFC 8414), XDS/proto transport, and SAML / refresh-token subjects.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ::http::request::Builder;
use ::http::{HeaderValue, Method, StatusCode, header};
use anyhow::{Context, anyhow, bail};
use base64::Engine;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use secrecy::{ExposeSecret, SecretString};

use crate::client::Client;
use crate::http::filters::BackendRequestTimeout;
use crate::http::{Body, Request, read_body_with_limit};
use crate::serdes::deser_key_from_file;
use crate::types::agent::Target;
use crate::*;

// Spec constants (draft-ietf-oauth-identity-assertion-authz-grant, RFC 8693, RFC 7523).
const GRANT_TYPE_TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const GRANT_TYPE_JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const TOKEN_TYPE_ID_JAG: &str = "urn:ietf:params:oauth:token-type:id-jag";
const TOKEN_TYPE_ID_TOKEN: &str = "urn:ietf:params:oauth:token-type:id_token";
const CLIENT_ASSERTION_TYPE_JWT_BEARER: &str =
	"urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

const TOKEN_RESPONSE_BODY_LIMIT: usize = 64 * 1024;
const DEFAULT_TOKEN_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Refresh a cached token this long before its real expiry, to avoid races with backend clocks.
const CACHE_EXPIRY_SKEW: Duration = Duration::from_secs(30);
/// Lifetime of the signed `private_key_jwt` client assertion.
const CLIENT_ASSERTION_LIFETIME: Duration = Duration::from_secs(300);

/// Configuration for the Identity Assertion (ID-JAG) backend auth grant.
#[apply(schema!)]
pub struct IdentityAssertion {
	/// The user's IdP authorization server, used for the RFC 8693 token exchange (step 1).
	pub idp: TokenEndpointConfig,
	/// The resource's authorization server, which exchanges the ID-JAG for an access token (step 2).
	pub resource_as: TokenEndpointConfig,
	/// Identifier of the resource authorization server. Sent as the `audience` of the token
	/// exchange; the issued ID-JAG is bound to it and may not be reused for another server.
	pub audience: String,
	/// Identifier of the protected resource (RFC 8707). If unset, defaults to the backend
	/// hostname (`https://<host>`).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub resource: Option<String>,
	/// Space-separated scopes to request. The authorization server may grant a subset.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scope: Option<String>,
	/// Upper bound on how long an issued access token is cached. The effective TTL is the
	/// smaller of this and the token's own `expires_in`. Defaults to the token's `expires_in`.
	#[serde(
		default,
		skip_serializing_if = "Option::is_none",
		with = "crate::serdes::serde_dur_option"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub cache_ttl: Option<Duration>,
	/// In-memory access-token cache, populated on first use. Not serialized.
	#[serde(skip)]
	#[cfg_attr(feature = "schema", schemars(skip))]
	cache: TokenCache,
}

/// A single OAuth token endpoint plus the client credentials used to authenticate to it.
#[apply(schema!)]
pub struct TokenEndpointConfig {
	/// Absolute URL of the token endpoint.
	pub token_endpoint: String,
	/// Client identifier registered at this authorization server.
	pub client_id: String,
	/// How the gateway authenticates as the client to this endpoint.
	pub client_auth: ClientAuth,
}

/// Client authentication method for a token endpoint.
#[apply(schema!)]
pub enum ClientAuth {
	/// `client_secret_basic`: HTTP Basic auth with the client id and secret.
	#[serde(rename_all = "camelCase")]
	ClientSecretBasic {
		#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
		#[serde(
			serialize_with = "ser_redact",
			deserialize_with = "deser_key_from_file"
		)]
		client_secret: SecretString,
	},
	/// `client_secret_post`: client id and secret sent as form parameters.
	#[serde(rename_all = "camelCase")]
	ClientSecretPost {
		#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
		#[serde(
			serialize_with = "ser_redact",
			deserialize_with = "deser_key_from_file"
		)]
		client_secret: SecretString,
	},
	/// `private_key_jwt`: a signed JWT client assertion (RFC 7523).
	#[serde(rename_all = "camelCase")]
	PrivateKeyJwt {
		/// PEM-encoded private signing key (RSA or EC, matching `alg`).
		#[cfg_attr(feature = "schema", schemars(with = "crate::serdes::FileOrInline"))]
		#[serde(
			serialize_with = "ser_redact",
			deserialize_with = "deser_key_from_file"
		)]
		signing_key: SecretString,
		/// Signing algorithm. Defaults to RS256.
		#[serde(default)]
		alg: SigningAlg,
		/// Optional `kid` header to include in the client assertion.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		kid: Option<String>,
	},
}

/// Signing algorithm for the `private_key_jwt` client assertion.
#[apply(schema_enum!)]
#[derive(Default)]
pub enum SigningAlg {
	#[default]
	#[serde(rename = "RS256")]
	Rs256,
	#[serde(rename = "RS384")]
	Rs384,
	#[serde(rename = "RS512")]
	Rs512,
	#[serde(rename = "ES256")]
	Es256,
	#[serde(rename = "ES384")]
	Es384,
}

impl SigningAlg {
	fn algorithm(self) -> Algorithm {
		match self {
			SigningAlg::Rs256 => Algorithm::RS256,
			SigningAlg::Rs384 => Algorithm::RS384,
			SigningAlg::Rs512 => Algorithm::RS512,
			SigningAlg::Es256 => Algorithm::ES256,
			SigningAlg::Es384 => Algorithm::ES384,
		}
	}

	fn encoding_key(self, pem: &[u8]) -> anyhow::Result<EncodingKey> {
		match self {
			SigningAlg::Rs256 | SigningAlg::Rs384 | SigningAlg::Rs512 => {
				EncodingKey::from_rsa_pem(pem).context("failed to load RSA signing key")
			},
			SigningAlg::Es256 | SigningAlg::Es384 => {
				EncodingKey::from_ec_pem(pem).context("failed to load EC signing key")
			},
		}
	}
}

#[derive(Clone, Debug, Default)]
struct TokenCache(Arc<Mutex<HashMap<CacheKey, CachedToken>>>);

type CacheKey = (String, String, String, String);

#[derive(Clone, Debug)]
struct CachedToken {
	header: HeaderValue,
	expires_at: Option<Instant>,
}

impl CachedToken {
	fn is_valid(&self) -> bool {
		match self.expires_at {
			None => true,
			Some(at) => Instant::now() < at,
		}
	}
}

/// JSON token-endpoint response, shared by both legs. `token_type` is intentionally not
/// asserted: the token exchange returns `N_A` per the spec, and the resource AS returns `Bearer`.
#[derive(serde::Deserialize)]
struct TokenResponse {
	access_token: String,
	#[serde(default)]
	expires_in: Option<u64>,
}

/// Entry point invoked from `apply_backend_auth`. Returns a sensitive `Authorization`
/// header value (`Bearer <token>`) for the backend request.
///
/// `subject_token` is the raw inbound JWT and `subject` its `sub` claim; both are extracted
/// by the caller so this future never borrows the (non-`Sync`) request across an await.
pub(super) async fn get_token(
	client: &Client,
	cfg: &IdentityAssertion,
	call_target: &Target,
	subject_token: &str,
	subject: &str,
) -> anyhow::Result<HeaderValue> {
	if subject_token.is_empty() {
		bail!("identityAssertion requires a non-empty inbound JWT as the subject token");
	}

	let resource = cfg.resource.clone().or_else(|| match call_target {
		Target::Hostname(host, _) => Some(format!("https://{host}")),
		_ => None,
	});
	let scope = cfg.scope.clone().unwrap_or_default();

	let key: CacheKey = (
		subject.to_string(),
		cfg.audience.clone(),
		scope,
		resource.clone().unwrap_or_default(),
	);

	// Fast path: return a still-valid cached token. The lock is never held across an await.
	if let Some(cached) = cfg.cache.0.lock().unwrap().get(&key)
		&& cached.is_valid()
	{
		return Ok(cached.header.clone());
	}

	// Step 1: exchange the inbound identity for an ID-JAG at the IdP.
	trace!(
		idp = %cfg.idp.token_endpoint,
		audience = %cfg.audience,
		"identityAssertion: exchanging identity for ID-JAG"
	);
	let id_jag = exchange_for_id_jag(client, cfg, subject_token, resource.as_deref()).await?;
	// Step 2: exchange the ID-JAG for a backend access token at the resource AS.
	trace!(
		resource_as = %cfg.resource_as.token_endpoint,
		"identityAssertion: exchanging ID-JAG for access token"
	);
	let access =
		exchange_id_jag_for_token(client, &cfg.resource_as, &id_jag, cfg.scope.as_deref()).await?;

	let mut header = HeaderValue::from_str(&format!("Bearer {}", access.access_token))
		.context("backend access token is not a valid header value")?;
	header.set_sensitive(true);

	let expires_at = access.expires_in.map(|secs| {
		let ttl = Duration::from_secs(secs)
			.saturating_sub(CACHE_EXPIRY_SKEW)
			.min(cfg.cache_ttl.unwrap_or(Duration::MAX));
		Instant::now() + ttl
	});
	cfg.cache.0.lock().unwrap().insert(
		key,
		CachedToken {
			header: header.clone(),
			expires_at,
		},
	);

	Ok(header)
}

async fn exchange_for_id_jag(
	client: &Client,
	cfg: &IdentityAssertion,
	subject_token: &str,
	resource: Option<&str>,
) -> anyhow::Result<String> {
	let mut form: Vec<(&str, String)> = vec![
		("grant_type", GRANT_TYPE_TOKEN_EXCHANGE.to_string()),
		("requested_token_type", TOKEN_TYPE_ID_JAG.to_string()),
		("audience", cfg.audience.clone()),
		("subject_token", subject_token.to_string()),
		("subject_token_type", TOKEN_TYPE_ID_TOKEN.to_string()),
	];
	if let Some(resource) = resource {
		form.push(("resource", resource.to_string()));
	}
	if let Some(scope) = &cfg.scope {
		form.push(("scope", scope.clone()));
	}
	let resp = post_token(client, &cfg.idp, form)
		.await
		.context("ID-JAG token exchange failed")?;
	Ok(resp.access_token)
}

async fn exchange_id_jag_for_token(
	client: &Client,
	resource_as: &TokenEndpointConfig,
	id_jag: &str,
	scope: Option<&str>,
) -> anyhow::Result<TokenResponse> {
	let mut form: Vec<(&str, String)> = vec![
		("grant_type", GRANT_TYPE_JWT_BEARER.to_string()),
		("assertion", id_jag.to_string()),
	];
	// The resource AS grants only what is explicitly requested here (it does not default to the
	// ID-JAG's scopes), so forward the configured scope. It must be a subset of the ID-JAG scopes.
	if let Some(scope) = scope {
		form.push(("scope", scope.to_string()));
	}
	post_token(client, resource_as, form)
		.await
		.context("ID-JAG access token request failed")
}

/// POST a form-encoded body to a token endpoint and parse the JSON response.
async fn post_token(
	client: &Client,
	ep: &TokenEndpointConfig,
	mut form: Vec<(&str, String)>,
) -> anyhow::Result<TokenResponse> {
	let builder = ::http::Request::builder()
		.method(Method::POST)
		.uri(ep.token_endpoint.as_str())
		.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
		.header(header::ACCEPT, "application/json");
	let builder = apply_client_auth(ep, &mut form, builder)?;

	let body = serde_urlencoded::to_string(&form).context("failed to encode token request form")?;
	let mut req: Request = builder
		.body(Body::from(body))
		.context("failed to build token request")?;
	req
		.extensions_mut()
		.insert(BackendRequestTimeout(DEFAULT_TOKEN_REQUEST_TIMEOUT));

	let resp = client
		.simple_call(req)
		.await
		.map_err(|e| anyhow!("token endpoint request failed: {e}"))?;
	let status = resp.status();
	trace!(endpoint = %ep.token_endpoint, %status, "identityAssertion: token endpoint responded");
	let body = read_body_with_limit(resp.into_body(), TOKEN_RESPONSE_BODY_LIMIT)
		.await
		.map_err(|e| anyhow!("failed to read token response body: {e}"))?;
	if status != StatusCode::OK {
		let detail = String::from_utf8_lossy(&body[..body.len().min(1024)]);
		trace!(endpoint = %ep.token_endpoint, %status, error = %detail, "identityAssertion: token endpoint error");
		bail!("token endpoint returned {status}: {detail}");
	}
	serde_json::from_slice::<TokenResponse>(&body).context("failed to decode token response")
}

/// Apply the configured client authentication to the request, mutating the form and/or headers.
fn apply_client_auth(
	ep: &TokenEndpointConfig,
	form: &mut Vec<(&str, String)>,
	builder: Builder,
) -> anyhow::Result<Builder> {
	Ok(match &ep.client_auth {
		ClientAuth::ClientSecretBasic { client_secret } => {
			let auth = format!(
				"Basic {}",
				base64::engine::general_purpose::STANDARD.encode(format!(
					"{}:{}",
					form_urlencode_component(&ep.client_id),
					form_urlencode_component(client_secret.expose_secret())
				))
			);
			builder.header(header::AUTHORIZATION, auth)
		},
		ClientAuth::ClientSecretPost { client_secret } => {
			form.push(("client_id", ep.client_id.clone()));
			form.push(("client_secret", client_secret.expose_secret().to_string()));
			builder
		},
		ClientAuth::PrivateKeyJwt {
			signing_key,
			alg,
			kid,
		} => {
			let assertion = sign_client_assertion(
				&ep.client_id,
				&ep.token_endpoint,
				signing_key,
				*alg,
				kid.as_deref(),
			)?;
			form.push((
				"client_assertion_type",
				CLIENT_ASSERTION_TYPE_JWT_BEARER.to_string(),
			));
			form.push(("client_assertion", assertion));
			builder
		},
	})
}

fn sign_client_assertion(
	client_id: &str,
	token_endpoint: &str,
	signing_key: &SecretString,
	alg: SigningAlg,
	kid: Option<&str>,
) -> anyhow::Result<String> {
	#[derive(serde::Serialize)]
	struct ClientAssertionClaims<'a> {
		iss: &'a str,
		sub: &'a str,
		aud: &'a str,
		jti: String,
		iat: u64,
		exp: u64,
	}

	let now = SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.context("system clock is before the unix epoch")?
		.as_secs();
	let claims = ClientAssertionClaims {
		iss: client_id,
		sub: client_id,
		aud: token_endpoint,
		jti: uuid::Uuid::new_v4().to_string(),
		iat: now,
		exp: now + CLIENT_ASSERTION_LIFETIME.as_secs(),
	};

	let mut header = Header::new(alg.algorithm());
	header.kid = kid.map(|k| k.to_string());
	let key = alg.encoding_key(signing_key.expose_secret().as_bytes())?;
	jsonwebtoken::encode(&header, &claims, &key).context("failed to sign client assertion")
}

fn form_urlencode_component(value: &str) -> String {
	url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
