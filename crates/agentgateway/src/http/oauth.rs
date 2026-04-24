#[derive(
	Debug, Clone, Copy, serde::Serialize, serde::Deserialize, Default, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum TokenEndpointAuth {
	#[default]
	ClientSecretBasic,
	ClientSecretPost,
	/// Public-client mode: no client authentication at the token endpoint
	/// (RFC 6749 §2.3.1 / OIDC Core §9 "none"). PKCE still protects the
	/// authorization code; see `crates/agentgateway/src/http/oidc/callback.rs`.
	None,
}

impl TokenEndpointAuth {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::ClientSecretBasic => "clientSecretBasic",
			Self::ClientSecretPost => "clientSecretPost",
			Self::None => "none",
		}
	}

	/// True when the method expects a `client_secret`. `None` is the only
	/// public-client method.
	pub fn requires_client_secret(self) -> bool {
		matches!(self, Self::ClientSecretBasic | Self::ClientSecretPost)
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

/// Pick a token-endpoint auth method from the IdP's advertised list, given
/// whether the caller has a `client_secret` to present.
///
/// Confidential callers (with a secret) prefer `client_secret_basic`, falling
/// back to `client_secret_post`. Public callers (no secret) require the IdP
/// to advertise `none`; otherwise the client is misconfigured and we surface
/// that explicitly instead of silently dropping the secret requirement.
pub(crate) fn parse_token_endpoint_auth_methods(
	methods: Option<Vec<String>>,
	has_client_secret: bool,
) -> Result<TokenEndpointAuth, String> {
	// Per OIDC Discovery §4.2 and RFC 8414 §2, the spec-defined default when
	// `token_endpoint_auth_methods_supported` is omitted is `client_secret_basic`.
	// A secretless (public) client that needs `none` must have it explicitly
	// advertised — otherwise we surface a clear error below instead of
	// silently programming an unauthenticated token request the IdP will reject.
	let methods = methods.unwrap_or_else(|| vec!["client_secret_basic".into()]);
	if has_client_secret {
		if methods.iter().any(|method| method == "client_secret_basic") {
			Ok(TokenEndpointAuth::ClientSecretBasic)
		} else if methods.iter().any(|method| method == "client_secret_post") {
			Ok(TokenEndpointAuth::ClientSecretPost)
		} else {
			Err(
				"IdP does not advertise clientSecretBasic or clientSecretPost; register the client as \
				 public (omit clientSecret and set tokenEndpointAuthMethod to None) or use a supported \
				 confidential method"
					.into(),
			)
		}
	} else if methods.iter().any(|method| method == "none") {
		Ok(TokenEndpointAuth::None)
	} else {
		Err(
			"OIDC client has no clientSecret but the IdP does not advertise the 'none' auth method; \
			 register the client as public at the IdP or provide a clientSecret"
				.into(),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::{
		TokenEndpointAuth, authorization_server_metadata_url, parse_token_endpoint_auth_methods,
	};

	#[test]
	fn authorization_server_metadata_url_supports_path_based_issuers() {
		assert_eq!(
			authorization_server_metadata_url("https://idp.example.com/application/o/myapp"),
			"https://idp.example.com/.well-known/oauth-authorization-server/application/o/myapp"
		);
	}

	#[test]
	fn parse_token_endpoint_auth_methods_prefers_basic() {
		let method = parse_token_endpoint_auth_methods(
			Some(vec![
				"private_key_jwt".into(),
				"client_secret_post".into(),
				"client_secret_basic".into(),
			]),
			true,
		)
		.expect("supported auth method");

		assert_eq!(method, TokenEndpointAuth::ClientSecretBasic);
	}

	#[test]
	fn parse_token_endpoint_auth_methods_rejects_missing_supported_values() {
		let err =
			parse_token_endpoint_auth_methods(Some(vec!["private_key_jwt".into(), "none".into()]), true);

		assert!(
			err.unwrap_err().contains("does not advertise"),
			"expected a message naming the missing advertised methods"
		);
	}

	#[test]
	fn parse_token_endpoint_auth_methods_selects_none_for_public_client() {
		let method =
			parse_token_endpoint_auth_methods(Some(vec!["none".into(), "private_key_jwt".into()]), false)
				.expect("public-client mode");

		assert_eq!(method, TokenEndpointAuth::None);
	}

	#[test]
	fn parse_token_endpoint_auth_methods_rejects_public_client_without_none_support() {
		let err = parse_token_endpoint_auth_methods(
			Some(vec![
				"client_secret_basic".into(),
				"client_secret_post".into(),
			]),
			false,
		);

		assert!(
			err
				.unwrap_err()
				.contains("does not advertise the 'none' auth method"),
			"expected guidance toward public-client registration"
		);
	}

	#[test]
	fn parse_token_endpoint_auth_methods_follows_spec_default_when_list_absent() {
		// OIDC Discovery §4.2 / RFC 8414 §2: omitted
		// `token_endpoint_auth_methods_supported` implies `client_secret_basic`.
		// Confidential clients get `basic`; public clients get a clear error
		// directing them to configure the IdP accordingly.
		assert_eq!(
			parse_token_endpoint_auth_methods(None, true).expect("confidential default"),
			TokenEndpointAuth::ClientSecretBasic,
		);
		let err = parse_token_endpoint_auth_methods(None, false)
			.expect_err("secretless client cannot assume `none` from a missing advertised list");
		assert!(
			err.contains("does not advertise the 'none' auth method"),
			"expected user-facing guidance, got: {err}"
		);
	}
}
