//! Token policies that may authenticate a request in place of an OIDC browser session.
//! See `LocalOidcConfig::allow_without_session`.

use super::AuthorizationLocation;
use crate::http::Request;
use crate::*;

/// A sibling authentication policy that may authenticate a request in place of a browser session.
#[apply(schema_enum!)]
pub enum SessionAlternative {
	JwtAuth,
	BasicAuth,
	ApiKey,
}

impl SessionAlternative {
	/// The policy's configuration key, as written in `allowWithoutSession`.
	pub fn as_str(&self) -> &'static str {
		match self {
			SessionAlternative::JwtAuth => "jwtAuth",
			SessionAlternative::BasicAuth => "basicAuth",
			SessionAlternative::ApiKey => "apiKey",
		}
	}
}

/// A token policy that `allowWithoutSession` can list.
pub trait SessionAlternativePolicy {
	/// The credential location when the policy is in `optional` mode: credentials are not
	/// required, but are rejected when invalid. `None` otherwise: `strict` requires credentials
	/// alongside the session, and `permissive` never rejects.
	fn optional_location(&self) -> Option<&AuthorizationLocation>;

	/// Whether the policy can stand in for a session at all (request-independent).
	fn qualifies_as_session_alternative(&self) -> bool {
		self
			.optional_location()
			.is_some_and(AuthorizationLocation::can_replace_session)
	}

	/// Whether the policy can stand in for a session on this request: it qualifies and the
	/// request carries its credential.
	fn has_optional_credential(&self, req: &Request) -> bool {
		self
			.optional_location()
			.is_some_and(|loc| loc.can_replace_session() && loc.extract(req).is_some())
	}
}

impl AuthorizationLocation {
	/// Whether a credential here is found deterministically and survives the stripping of reserved
	/// OIDC cookies: a header (other than `Cookie`), a query parameter, or a non-reserved cookie.
	/// Not a CEL `expression`.
	pub fn can_replace_session(&self) -> bool {
		match self {
			AuthorizationLocation::Header { name, .. } => name != ::http::header::COOKIE,
			AuthorizationLocation::QueryParameter { .. } => true,
			AuthorizationLocation::Cookie { name } => {
				!name.starts_with(crate::http::oidc::RESERVED_COOKIE_PREFIX)
			},
			AuthorizationLocation::Expression(_) => false,
		}
	}
}

/// Request extension recording which token policies authenticated the request. Set by `jwtAuth`,
/// `basicAuth`, and `apiKey` on success; the proxy resets and checks it around a phase's siblings
/// when OIDC deferred to them.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenAuthenticated {
	jwt_auth: bool,
	basic_auth: bool,
	api_key: bool,
}

impl TokenAuthenticated {
	pub fn record(req: &mut Request, policy: SessionAlternative) {
		let marker = req.extensions_mut().get_or_insert_default::<Self>();
		match policy {
			SessionAlternative::JwtAuth => marker.jwt_auth = true,
			SessionAlternative::BasicAuth => marker.basic_auth = true,
			SessionAlternative::ApiKey => marker.api_key = true,
		}
	}

	pub fn contains(&self, policy: SessionAlternative) -> bool {
		match policy {
			SessionAlternative::JwtAuth => self.jwt_auth,
			SessionAlternative::BasicAuth => self.basic_auth,
			SessionAlternative::ApiKey => self.api_key,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn can_replace_session_excludes_locations_oidc_rewrites_or_cannot_probe() {
		let header = |name: &str| AuthorizationLocation::Header {
			name: ::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
			prefix: None,
		};
		let cookie = |name: &str| AuthorizationLocation::Cookie { name: name.into() };
		assert!(AuthorizationLocation::bearer_header().can_replace_session());
		assert!(AuthorizationLocation::QueryParameter { name: "key".into() }.can_replace_session());
		assert!(cookie("token").can_replace_session());
		assert!(!cookie("agw_oidc_s_token").can_replace_session());
		assert!(!header("cookie").can_replace_session());
		let expression = crate::cel::Expression::new_strict("'x'").unwrap();
		assert!(!AuthorizationLocation::Expression(Arc::new(expression)).can_replace_session());
	}
}
