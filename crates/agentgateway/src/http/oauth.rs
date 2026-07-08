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
