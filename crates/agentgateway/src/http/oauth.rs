#[derive(
	Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

#[cfg(test)]
mod tests {
	use super::{
		TokenEndpointAuth, authorization_server_metadata_url, openid_configuration_metadata_url,
		parse_token_endpoint_auth_methods,
	};

	#[test]
	fn openid_configuration_metadata_url_normalizes_trailing_slashes() {
		assert_eq!(
			openid_configuration_metadata_url("http://keycloak:8080/realms/test/"),
			"http://keycloak:8080/realms/test/.well-known/openid-configuration"
		);
	}

	#[test]
	fn authorization_server_metadata_url_supports_path_based_issuers() {
		assert_eq!(
			authorization_server_metadata_url("https://idp.example.com/application/o/myapp"),
			"https://idp.example.com/.well-known/oauth-authorization-server/application/o/myapp"
		);
	}

	#[test]
	fn authorization_server_metadata_url_normalizes_root_issuers() {
		assert_eq!(
			authorization_server_metadata_url("https://idp.example.com/"),
			"https://idp.example.com/.well-known/oauth-authorization-server"
		);
	}

	#[test]
	fn parse_token_endpoint_auth_methods_prefers_basic() {
		let method = parse_token_endpoint_auth_methods(Some(vec![
			"private_key_jwt".into(),
			"client_secret_post".into(),
			"client_secret_basic".into(),
		]))
		.expect("supported auth method");

		assert_eq!(method, TokenEndpointAuth::ClientSecretBasic);
	}

	#[test]
	fn parse_token_endpoint_auth_methods_rejects_missing_supported_values() {
		let err =
			parse_token_endpoint_auth_methods(Some(vec!["private_key_jwt".into(), "none".into()]));

		assert_eq!(
			err.unwrap_err(),
			"token endpoint auth methods must include clientSecretBasic or clientSecretPost"
		);
	}
}
