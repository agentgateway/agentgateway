use std::io::Write;

use base64::prelude::BASE64_STANDARD;
use base64::write::EncoderStringWriter;
use secrecy::{ExposeSecret, SecretString};

use crate::{apply, schema};

pub(crate) const GRANT_TYPE_TOKEN_EXCHANGE: &str =
	"urn:ietf:params:oauth:grant-type:token-exchange";
pub(crate) const GRANT_TYPE_JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";

pub(crate) const CLIENT_ASSERTION_TYPE_JWT_BEARER: &str =
	"urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

pub(crate) const TOKEN_TYPE_ACCESS: &str = "urn:ietf:params:oauth:token-type:access_token";
pub(crate) const TOKEN_TYPE_ID: &str = "urn:ietf:params:oauth:token-type:id_token";
pub(crate) const TOKEN_TYPE_ID_JAG: &str = "urn:ietf:params:oauth:token-type:id-jag";
pub(crate) const TOKEN_TYPE_JWT: &str = "urn:ietf:params:oauth:token-type:jwt";

#[apply(schema!)]
#[derive(Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum TokenEndpointAuth {
	#[default]
	ClientSecretBasic,
	ClientSecretPost,
}

impl TokenEndpointAuth {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::ClientSecretBasic => "clientSecretBasic",
			Self::ClientSecretPost => "clientSecretPost",
		}
	}
}

pub(crate) fn openid_configuration_metadata_url(issuer: &str) -> String {
	format!(
		"{}/.well-known/openid-configuration",
		issuer.trim_end_matches('/')
	)
}

pub(crate) fn authorization_server_metadata_url(issuer: &str) -> String {
	match url::Url::parse(issuer) {
		Ok(parsed) => {
			let origin = parsed.origin().ascii_serialization();
			let path = parsed.path();
			if path == "/" {
				format!("{origin}/.well-known/oauth-authorization-server")
			} else {
				format!("{origin}/.well-known/oauth-authorization-server{path}")
			}
		},
		Err(_) => {
			let normalized = issuer.trim_end_matches('/');
			format!("{normalized}/.well-known/oauth-authorization-server")
		},
	}
}

/// OAuth endpoints for a Microsoft Entra ID (Azure AD) tenant, derived from the configured
/// issuer.
///
/// Entra apps can be configured to mint v1 tokens (`iss` = `https://sts.windows.net/{tenant}/`)
/// or v2 tokens (`iss` = `https://login.microsoftonline.com/{tenant}/v2.0`). Either form can be
/// configured as the issuer; the interactive OAuth endpoints always live on the login host
/// under `oauth2/v2.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntraEndpoints {
	pub openid_configuration: String,
	pub authorization_endpoint: String,
	pub token_endpoint: String,
	pub jwks_uri: String,
}

pub(crate) fn entra_endpoints(issuer: &str) -> Result<EntraEndpoints, String> {
	let parsed =
		url::Url::parse(issuer).map_err(|e| format!("invalid Entra issuer URL {issuer:?}: {e}"))?;
	let host = parsed
		.host_str()
		.ok_or_else(|| format!("Entra issuer URL {issuer:?} has no host"))?;
	let tenant = parsed
		.path_segments()
		.and_then(|mut segments| segments.find(|s| !s.is_empty()).map(ToString::to_string))
		.ok_or_else(|| {
			format!("Entra issuer URL {issuer:?} has no tenant path segment (expected https://login.microsoftonline.com/<tenant>/v2.0 or https://sts.windows.net/<tenant>/)")
		})?;
	// v1 tokens are issued by sts.windows.net, but the interactive endpoints live on the
	// login host. Other hosts (e.g. sovereign clouds like login.microsoftonline.us) pass through.
	let login_host = if host.eq_ignore_ascii_case("sts.windows.net") {
		"login.microsoftonline.com"
	} else {
		host
	};
	let authority = format!("https://{login_host}/{tenant}");
	Ok(EntraEndpoints {
		openid_configuration: format!("{authority}/v2.0/.well-known/openid-configuration"),
		authorization_endpoint: format!("{authority}/oauth2/v2.0/authorize"),
		token_endpoint: format!("{authority}/oauth2/v2.0/token"),
		jwks_uri: format!("{authority}/discovery/v2.0/keys"),
	})
}

pub(crate) fn parse_token_endpoint_auth_methods(
	methods: Option<Vec<String>>,
) -> Result<TokenEndpointAuth, String> {
	let methods = methods.unwrap_or_else(|| vec!["client_secret_basic".into()]);
	if methods.iter().any(|method| method == "client_secret_basic") {
		Ok(TokenEndpointAuth::ClientSecretBasic)
	} else if methods.iter().any(|method| method == "client_secret_post") {
		Ok(TokenEndpointAuth::ClientSecretPost)
	} else {
		Err("token endpoint auth methods must include clientSecretBasic or clientSecretPost".into())
	}
}

/// build `base64(urlencode(client_id) + ":" + urlencode(client_secret))` credential
pub(crate) fn encode_client_secret_basic(client_id: &str, client_secret: &SecretString) -> String {
	use url::form_urlencoded::byte_serialize;
	let mut encoded = EncoderStringWriter::new(&BASE64_STANDARD);
	for p in byte_serialize(client_id.as_bytes()) {
		encoded.write_all(p.as_bytes()).unwrap();
	}
	encoded.write_all(b":").unwrap();
	for p in byte_serialize(client_secret.expose_secret().as_bytes()) {
		encoded.write_all(p.as_bytes()).unwrap();
	}
	encoded.into_inner()
}

pub(crate) fn format_token_endpoint_error_body(body: &[u8], limit: usize) -> String {
	let mut out = String::with_capacity(body.len().min(limit));
	let mut truncated = false;
	for ch in String::from_utf8_lossy(body).chars() {
		let ch = if ch.is_control() { ' ' } else { ch };
		if out.len() + ch.len_utf8() > limit {
			truncated = true;
			break;
		}
		out.push(ch);
	}
	if truncated {
		out.push_str("...");
	}
	out
}

#[cfg(test)]
mod tests {
	use base64::Engine;
	use base64::prelude::BASE64_STANDARD;
	use rstest::rstest;

	use super::*;

	#[test]
	fn entra_endpoints_from_v2_issuer() {
		let endpoints = entra_endpoints(
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/v2.0",
		)
		.expect("valid v2 issuer");
		assert_eq!(
			endpoints.openid_configuration,
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/v2.0/.well-known/openid-configuration"
		);
		assert_eq!(
			endpoints.authorization_endpoint,
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/oauth2/v2.0/authorize"
		);
		assert_eq!(
			endpoints.token_endpoint,
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/oauth2/v2.0/token"
		);
		assert_eq!(
			endpoints.jwks_uri,
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/discovery/v2.0/keys"
		);
	}

	#[test]
	fn entra_endpoints_from_v1_issuer() {
		// v1 tokens have iss=https://sts.windows.net/{tenant}/; the OAuth endpoints still live
		// on login.microsoftonline.com.
		let endpoints =
			entra_endpoints("https://sts.windows.net/11111111-2222-3333-4444-555555555555/")
				.expect("valid v1 issuer");
		assert_eq!(
			endpoints.token_endpoint,
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/oauth2/v2.0/token"
		);
		assert_eq!(
			endpoints.jwks_uri,
			"https://login.microsoftonline.com/11111111-2222-3333-4444-555555555555/discovery/v2.0/keys"
		);
	}

	#[test]
	fn entra_endpoints_preserves_sovereign_cloud_hosts() {
		let endpoints = entra_endpoints("https://login.microsoftonline.us/tenant-id/v2.0")
			.expect("valid sovereign cloud issuer");
		assert_eq!(
			endpoints.authorization_endpoint,
			"https://login.microsoftonline.us/tenant-id/oauth2/v2.0/authorize"
		);
	}

	#[test]
	fn entra_endpoints_rejects_issuer_without_tenant() {
		assert!(entra_endpoints("https://login.microsoftonline.com").is_err());
		assert!(entra_endpoints("not a url").is_err());
	}

	#[test]
	fn authorization_server_metadata_url_supports_path_based_issuers() {
		assert_eq!(
			authorization_server_metadata_url("https://idp.example.com/application/o/myapp"),
			"https://idp.example.com/.well-known/oauth-authorization-server/application/o/myapp"
		);
	}

	#[rstest]
	#[case(
		Some(vec![
			"private_key_jwt".into(),
			"client_secret_post".into(),
			"client_secret_basic".into(),
		]),
		Ok(TokenEndpointAuth::ClientSecretBasic)
	)]
	#[case(
		Some(vec!["private_key_jwt".into(), "none".into()]),
		Err("token endpoint auth methods must include clientSecretBasic or clientSecretPost")
	)]
	fn parse_token_endpoint_auth_methods_cases(
		#[case] methods: Option<Vec<String>>,
		#[case] expected: Result<TokenEndpointAuth, &str>,
	) {
		let actual = parse_token_endpoint_auth_methods(methods);
		let expected = expected.map_err(str::to_string);
		assert_eq!(actual, expected);
	}

	#[test]
	fn encode_client_secret_basic_form_encodes_credentials() {
		assert_eq!(
			format!(
				"Basic {}",
				encode_client_secret_basic("gw client", &"s3:cr3t".into())
			),
			format!("Basic {}", BASE64_STANDARD.encode("gw+client:s3%3Acr3t"))
		);
	}

	#[test]
	fn format_token_endpoint_error_body_sanitizes_and_truncates() {
		assert_eq!(
			format_token_endpoint_error_body("bad\nthing😬tail".as_bytes(), 12),
			"bad thing..."
		);
	}
}
