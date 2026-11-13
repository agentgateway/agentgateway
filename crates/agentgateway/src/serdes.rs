use std::path::PathBuf;

pub use agent_core::serdes::*;
use anyhow::Context;
use openapiv3::OpenAPI;
use serde::de::DeserializeOwned;

use crate::resource_manager::{ResourceFetcher, ResourceKind, ResourceRef};

define_schema_aliases!();

/// Loads [`FileOrInline`] values through the resource manager so file-backed
/// values participate in config reloads.
pub async fn load_file_or_inline(
	value: &FileOrInline,
	resources: &ResourceFetcher,
) -> anyhow::Result<String> {
	match value {
		FileOrInline::Inline(value) => Ok(value.clone()),
		FileOrInline::File { file } => {
			let bytes = resources.fetch(ResourceRef::File(file.clone())).await?;
			String::from_utf8(bytes.to_vec()).context("resource is not valid UTF-8")
		},
	}
}

/// [`load_file_or_inline`] for serde hooks, through the fetcher of the enclosing parse.
pub fn load_file_or_inline_during_parse(value: &FileOrInline) -> anyhow::Result<String> {
	let resources = crate::resource_manager::parse_fetcher();
	futures::executor::block_on(load_file_or_inline(value, &resources))
}

pub fn deser_watched_secret<'de, D>(deserializer: D) -> Result<secrecy::SecretString, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let input = <FileOrInline as serde::Deserialize>::deserialize(deserializer)?;
	load_file_or_inline_during_parse(&input)
		.map(|s| secrecy::SecretString::from(s.trim().to_string()))
		.map_err(serde::de::Error::custom)
}

pub fn deser_watched_secret_option<'de, D>(
	deserializer: D,
) -> Result<Option<secrecy::SecretString>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let Some(input) = <Option<FileOrInline> as serde::Deserialize>::deserialize(deserializer)? else {
		return Ok(None);
	};
	load_file_or_inline_during_parse(&input)
		.map(|s| Some(secrecy::SecretString::from(s.trim().to_string())))
		.map_err(serde::de::Error::custom)
}

#[derive(Debug, Clone, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(untagged)]
pub enum FileInlineOrRemote {
	File {
		/// Path to a file on disk to load the value from.
		file: PathBuf,
	},
	Inline(String),
	Remote {
		#[serde(deserialize_with = "de_parse")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		url: http::Uri,
	},
}

impl FileInlineOrRemote {
	/// Keep a remote JWKS outage local to its verifier. Parsing errors and local
	/// file errors still reject the config; an unavailable endpoint has no keys.
	pub async fn load_jwks(
		&self,
		resources: &ResourceFetcher,
	) -> anyhow::Result<jsonwebtoken::jwk::JwkSet> {
		let Self::Remote { url } = self else {
			return self.load(resources, ResourceKind::Jwks).await;
		};
		anyhow::ensure!(
			matches!(url.scheme_str(), Some("http" | "https")) && url.host().is_some(),
			"JWKS URL must be an absolute HTTP or HTTPS URL: {url}"
		);
		let resource = ResourceRef::Http {
			url: url.clone(),
			kind: ResourceKind::Jwks,
		};
		match resources.fetch(resource).await {
			Ok(bytes) => serde_json::from_slice(&bytes).map_err(Into::into),
			Err(e) => {
				tracing::warn!(%url, error = %e, "JWKS endpoint unavailable; no verification keys loaded");
				Ok(jsonwebtoken::jwk::JwkSet { keys: Vec::new() })
			},
		}
	}

	pub async fn load<T: DeserializeOwned>(
		&self,
		resources: &ResourceFetcher,
		kind: ResourceKind,
	) -> anyhow::Result<T> {
		let s = self.load_string(resources, kind).await?;
		serde_json::from_str(&s).map_err(Into::into)
	}

	pub async fn load_openapi_schema(&self, resources: &ResourceFetcher) -> anyhow::Result<OpenAPI> {
		let s = self.load_string(resources, ResourceKind::OpenApi).await?;
		stacker::grow(2 * 1024 * 1024, || yaml::from_str::<OpenAPI>(s.as_str()))
	}

	async fn load_string(
		&self,
		resources: &ResourceFetcher,
		kind: ResourceKind,
	) -> anyhow::Result<String> {
		Ok(match self {
			FileInlineOrRemote::Inline(s) => s.clone(),
			FileInlineOrRemote::File { .. } | FileInlineOrRemote::Remote { .. } => {
				let bytes = resources
					.fetch(self.as_resource_ref(kind).expect("resource ref"))
					.await?;
				String::from_utf8(bytes.to_vec())?
			},
		})
	}

	fn as_resource_ref(&self, kind: ResourceKind) -> Option<ResourceRef> {
		match self {
			FileInlineOrRemote::File { file } => Some(ResourceRef::File(file.clone())),
			FileInlineOrRemote::Inline(_) => None,
			FileInlineOrRemote::Remote { url } => Some(ResourceRef::Http {
				url: url.clone(),
				kind,
			}),
		}
	}
}

#[cfg(test)]
mod jwks_tests {
	use serde_json::json;
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	use super::ResourceFetcher;
	use crate::http::jwt::LocalJwtConfig;
	use crate::http::jwt::tests::build_signed_token_with_payload;
	use crate::http::oidc::{LocalOidcConfig, PolicyId};
	use crate::http::sessionpersistence::Encoder;

	fn jwks_test_fetcher() -> ResourceFetcher {
		ResourceFetcher::direct(crate::client::Client::new(
			&crate::client::Config {
				resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
				resolver_opts: hickory_resolver::config::ResolverOpts::default(),
				max_ttl: None,
			},
			None,
			crate::BackendConfig::default(),
			None,
		))
	}

	#[tokio::test]
	async fn remote_jwks_outage_preserves_healthy_issuers_and_rejects_unverified_tokens() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/jwks"))
			.respond_with(ResponseTemplate::new(503))
			.mount(&server)
			.await;
		let healthy_jwks = json!({"keys": [{
			"kty": "EC", "kid": "shared-key", "crv": "P-256", "alg": "ES256",
			"x": "WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk",
			"y": "xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"
		}]});
		let healthy_token = build_signed_token_with_payload(
			"shared-key",
			json!({"iss": "healthy", "aud": "aud", "exp": 4102444800_u64}),
		);
		// Same key ID and signing key must not let an unavailable issuer borrow trust.
		let unavailable_token = build_signed_token_with_payload(
			"shared-key",
			json!({"iss": "unavailable", "aud": "aud", "exp": 4102444800_u64}),
		);
		let resources = jwks_test_fetcher();
		let config: LocalJwtConfig = serde_json::from_value(json!({
			"mode": "strict",
			"providers": [
				{"issuer": "unavailable", "jwks": {"url": format!("{}/jwks", server.uri())}},
				{"issuer": "healthy", "jwks": healthy_jwks.to_string()}
			]
		}))
		.unwrap();
		let jwt = config.try_into(&resources).await.unwrap();
		assert!(jwt.validate_claims(&healthy_token).is_ok());
		let mut request = ::http::Request::builder()
			.header("authorization", format!("Bearer {unavailable_token}"))
			.body(crate::http::Body::empty())
			.unwrap();
		assert!(jwt.apply(None, &mut request).await.is_err());
	}

	#[tokio::test]
	async fn jwks_outage_handling_does_not_hide_configuration_errors() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_string("not a key set"))
			.mount(&server)
			.await;
		let resources = jwks_test_fetcher();
		for jwks in [
			json!({"url": "/relative-jwks"}),
			json!({"url": format!("{}/jwks", server.uri())}),
		] {
			let config: LocalJwtConfig = serde_json::from_value(json!({
				"mode": "strict", "issuer": "issuer", "jwks": jwks
			}))
			.unwrap();
			assert!(config.try_into(&resources).await.is_err());
		}
	}

	#[tokio::test]
	async fn local_oidc_jwks_outage_keeps_id_token_verification_required() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(503))
			.mount(&server)
			.await;
		let config: LocalOidcConfig = serde_json::from_value(json!({
			"issuer": "https://issuer.example.com",
			"authorizationEndpoint": "https://issuer.example.com/authorize",
			"tokenEndpoint": "https://issuer.example.com/token",
			"jwks": {"url": format!("{}/jwks", server.uri())},
			"clientId": "client-id",
			"clientSecret": "client-secret",
			"redirectURI": "https://app.example.com/oauth/callback"
		}))
		.unwrap();
		let resources = jwks_test_fetcher();
		let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
		let encoder = Encoder::aes(key).unwrap();
		let policy = config
			.compile(&resources, PolicyId::policy("jwks-outage"), &encoder)
			.await
			.expect("remote JWKS outage should not reject the OIDC policy");
		let token = build_signed_token_with_payload(
			"kid-1",
			json!({"iss": "https://issuer.example.com", "aud": "client-id", "exp": 4102444800_u64}),
		);
		assert!(
			policy
				.provider
				.id_token_validator
				.validate_claims(&token)
				.is_err()
		);
	}
}
