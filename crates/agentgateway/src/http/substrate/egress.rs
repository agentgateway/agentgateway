use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::http::{HeaderName, HeaderValue};
use ipnet::IpNet;
use quick_cache::sync::Cache;
use tonic::Code;

use super::{ActorIdentity, ActorRef, TRACE_POLICY_KIND};
use crate::http::{PolicyResponse, Request};
use crate::proxy::httpproxy::PolicyClient;
use crate::proxy::{ProxyError, ProxyResponse};
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::{cel, *};

const DEFAULT_CREDENTIAL_CACHE_CAPACITY: usize = 8192;
const DEFAULT_CREDENTIAL_CACHE_TTL: Duration = Duration::from_secs(300);

/// Retrieves and enforces the current Substrate egress policy for each request.
#[apply(schema!)]
pub struct SubstrateEgress {
	/// Backend that receives GetActorEgressPolicy calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
	/// Credential providers available to secret-backed egress effects, keyed by
	/// the authority in a `substrate-secret://` URI.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub credential_providers: Vec<CredentialProvider>,
	#[serde(skip, default = "default_credential_cache")]
	#[cfg_attr(feature = "schema", schemars(skip))]
	credential_cache: CredentialCache,
}

/// An inline credential-provider backend selected by credential URI authority.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CredentialProvider {
	/// Exact credential URI authority handled by this provider, such as `kubernetes.io`.
	#[serde(rename = "uriAuthority")]
	pub uri_authority: String,
	/// Backend that resolves credentials and policies used when connecting to it.
	pub target: SimpleBackendReferenceWithPolicies,
}

#[derive(Debug, Clone)]
struct CredentialCache {
	entries: Arc<Cache<CredentialCacheKey, CachedCredential>>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct CredentialCacheKey {
	actor_identity: String,
	uri: String,
}

#[derive(Clone)]
struct CachedCredential {
	secret: Vec<u8>,
	fetched_at: Instant,
}

impl CredentialCache {
	fn new(capacity: usize) -> Self {
		Self {
			entries: Arc::new(Cache::new(capacity)),
		}
	}

	fn get(&self, key: &CredentialCacheKey, now: Instant, ttl: Duration) -> Option<Vec<u8>> {
		if let Some(entry) = self.entries.get(key) {
			if now.duration_since(entry.fetched_at) <= ttl {
				return Some(entry.secret);
			}
			self
				.entries
				.remove_if(key, |entry| now.duration_since(entry.fetched_at) > ttl);
		}
		None
	}

	fn insert(&self, key: CredentialCacheKey, secret: Vec<u8>, now: Instant) {
		self.entries.insert(
			key,
			CachedCredential {
				secret,
				fetched_at: now,
			},
		);
	}
}

fn default_credential_cache() -> CredentialCache {
	CredentialCache::new(DEFAULT_CREDENTIAL_CACHE_CAPACITY)
}

impl RequestPolicyTrait for SubstrateEgress {
	async fn apply(
		&self,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyResponse> {
		let identity = req
			.extensions()
			.get::<ActorIdentity>()
			.cloned()
			.ok_or_else(|| {
				ProxyError::SubstrateEgressDenied("missing CONNECT-authorized actor identity".to_owned())
			})?;
		let actor = ActorRef {
			atespace: identity.atespace.clone(),
			name: identity.actor_name.clone(),
		};
		log.ate_actor_name = Some(actor.name.clone());
		log.ate_actor_uid = Some(identity.actor_uid.clone());
		log.ate_atespace = Some(actor.atespace.clone());
		let channel = self
			.target
			.grpc_channel(client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate));
		let mut control = protos::ateapi::control_client::ControlClient::new(channel);
		let policy = crate::proxy::dtrace::scope_future(
			Some(TRACE_POLICY_KIND),
			control.get_actor_egress_policy(protos::ateapi::GetActorEgressPolicyRequest {
				actor: Some(protos::ateapi::ObjectRef {
					atespace: actor.atespace,
					name: actor.name,
				}),
			}),
		)
		.await;
		let policy = match policy {
			Ok(response) => response.into_inner(),
			Err(status) if matches!(status.code(), Code::Unavailable | Code::DeadlineExceeded) => {
				return Err(
					ProxyError::SubstrateEgressUnavailable(format!(
						"actor egress policy unavailable: {status}"
					))
					.into(),
				);
			},
			Err(status) => {
				return Err(
					ProxyError::SubstrateEgressDenied(format!("actor egress policy denied: {status}")).into(),
				);
			},
		};
		let matched_rule = matching_rule(&policy, req)?;
		self
			.apply_effects(client, &identity, matched_rule, req)
			.await?;
		Ok(PolicyResponse::default())
	}
}

impl SubstrateEgress {
	async fn apply_effects(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
		rule: &protos::ateapi::EgressRule,
		req: &mut Request,
	) -> Result<(), ProxyResponse> {
		let Some(effects) = rule
			.hostnames
			.as_ref()
			.and_then(|hostnames| hostnames.effects.as_ref())
		else {
			return Ok(());
		};
		for injection in &effects.inject_static_headers {
			let provider = self.provider_for_uri(&injection.credential_uri)?;
			let secret = self
				.credential(client, identity, provider, &injection.credential_uri)
				.await?;
			let (name, value) = credential_header(injection, secret)?;
			req.headers_mut().insert(name, value);
		}
		Ok(())
	}

	async fn credential(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
		provider: &CredentialProvider,
		uri: &str,
	) -> Result<Vec<u8>, ProxyResponse> {
		let actor_identity = actor_spiffe_uri(&identity.atespace, &identity.actor_name);
		let key = CredentialCacheKey {
			actor_identity: actor_identity.clone(),
			uri: uri.to_owned(),
		};
		if let Some(secret) =
			self
				.credential_cache
				.get(&key, Instant::now(), DEFAULT_CREDENTIAL_CACHE_TTL)
		{
			return Ok(secret);
		}

		let channel = provider
			.target
			.grpc_channel(client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate));
		let mut provider =
			protos::credprovider::credential_provider_client::CredentialProviderClient::new(channel);
		let response = provider
			.request_secret(protos::credprovider::RequestSecretRequest {
				uri: uri.to_owned(),
				context: Some(protos::credprovider::SecretRequestContext { actor_identity }),
			})
			.await
			.map_err(|status| credential_provider_error(uri, status))?
			.into_inner();
		let secret = credential_secret(response.secret)?;
		self
			.credential_cache
			.insert(key, secret.clone(), Instant::now());
		Ok(secret)
	}

	fn provider_for_uri(&self, uri: &str) -> Result<&CredentialProvider, ProxyResponse> {
		let name = provider_name(uri)
			.ok_or_else(|| ProxyError::SubstrateEgressDenied(format!("invalid credential URI: {uri}")))?;
		self
			.credential_providers
			.iter()
			.find(|provider| provider.uri_authority == name)
			.ok_or_else(|| {
				ProxyError::SubstrateEgressDenied(format!("no credential provider configured for {name}"))
			})
			.map_err(Into::into)
	}
}

fn provider_name(uri: &str) -> Option<&str> {
	let authority = uri.strip_prefix("substrate-secret://")?.split('/').next()?;
	(!authority.is_empty() && !authority.contains(['?', '#', '@', ':'])).then_some(authority)
}

fn actor_spiffe_uri(atespace: &str, actor_name: &str) -> String {
	// Matches Substrate's actorspiffe.Parse contract in
	// internal/actorspiffe/actorspiffe.go.
	format!("spiffe://substrate-actor.local/atespace/{atespace}/actor/{actor_name}")
}

fn credential_header(
	injection: &protos::ateapi::CredentialHeaderInjection,
	secret: Vec<u8>,
) -> Result<(HeaderName, HeaderValue), ProxyResponse> {
	let name = HeaderName::from_str(&injection.header).map_err(|error| {
		ProxyError::SubstrateEgressDenied(format!("invalid credential header: {error}"))
	})?;
	if protected_credential_header(&name) {
		return Err(
			ProxyError::SubstrateEgressDenied(format!(
				"credential effects cannot modify protected header {name}"
			))
			.into(),
		);
	}
	let secret = credential_secret(secret)?;
	let mut value = injection.prefix.as_bytes().to_vec();
	value.extend(&secret);
	let mut value = HeaderValue::from_bytes(&value).map_err(|error| {
		ProxyError::SubstrateEgressUnavailable(format!("credential header value is invalid: {error}"))
	})?;
	value.set_sensitive(true);
	Ok((name, value))
}

fn protected_credential_header(name: &HeaderName) -> bool {
	matches!(
		name.as_str(),
		"host"
			| "content-length"
			| "connection"
			| "keep-alive"
			| "proxy-authenticate"
			| "proxy-authorization"
			| "te"
			| "trailer"
			| "transfer-encoding"
			| "upgrade"
	)
}

fn credential_provider_error(uri: &str, status: tonic::Status) -> ProxyError {
	let provider = provider_name(uri).unwrap_or("unknown");
	match status.code() {
		Code::Unavailable | Code::DeadlineExceeded => ProxyError::SubstrateEgressUnavailable(format!(
			"credential provider {provider} unavailable: {status}"
		)),
		_ => {
			ProxyError::SubstrateEgressDenied(format!("credential provider {provider} denied: {status}"))
		},
	}
}

fn credential_secret(secret: Vec<u8>) -> Result<Vec<u8>, ProxyResponse> {
	let secret = secret.strip_suffix(b"\n").unwrap_or(&secret);
	let secret = secret.strip_suffix(b"\r").unwrap_or(secret);
	if secret.is_empty() || secret.iter().any(|byte| byte.is_ascii_control()) {
		return Err(
			ProxyError::SubstrateEgressUnavailable(
				"credential provider returned an unusable secret".to_owned(),
			)
			.into(),
		);
	}
	Ok(secret.to_vec())
}

fn matching_rule<'a>(
	policy: &'a protos::ateapi::EgressPolicy,
	req: &Request,
) -> Result<&'a protos::ateapi::EgressRule, ProxyResponse> {
	let destination = req
		.extensions()
		.get::<cel::DestinationContext>()
		.ok_or_else(|| {
			ProxyError::SubstrateEgressDenied("missing egress destination context".to_owned())
		})?;
	for rule in &policy.rules {
		if rule_matches(rule, destination)? {
			return Ok(rule);
		}
	}
	Err(ProxyError::SubstrateEgressDenied("actor egress policy denied destination".to_owned()).into())
}

fn rule_matches(
	rule: &protos::ateapi::EgressRule,
	destination: &cel::DestinationContext,
) -> Result<bool, ProxyResponse> {
	if let Some(hostnames) = &rule.hostnames {
		return Ok(destination.hostname.as_deref().is_some_and(|hostname| {
			hostnames
				.patterns
				.iter()
				.any(|pattern| hostname_matches(pattern, hostname))
		}));
	}
	if let Some(ip_blocks) = &rule.ip_blocks {
		return ip_blocks.cidrs.iter().try_fold(false, |matches, cidr| {
			if matches {
				return Ok(true);
			}
			let network = cidr.parse::<IpNet>().map_err(|error| {
				ProxyError::SubstrateEgressDenied(format!("invalid actor egress CIDR: {error}"))
			})?;
			Ok(network.contains(&destination.address))
		});
	}
	Ok(rule.all.is_some())
}

fn hostname_matches(pattern: &str, hostname: &str) -> bool {
	if let Some(suffix) = pattern.strip_prefix("*.") {
		let Some(prefix) = hostname.strip_suffix(suffix) else {
			return false;
		};
		let Some(label) = prefix.strip_suffix('.') else {
			return false;
		};
		!label.is_empty() && !label.contains('.')
	} else {
		pattern == hostname
	}
}

#[cfg(test)]
mod tests {
	use std::net::IpAddr;

	use super::*;

	fn request(address: &str, hostname: Option<&str>) -> Request {
		let address = address.parse::<IpAddr>().unwrap();
		let mut request = Request::new(crate::http::Body::empty());
		request.extensions_mut().insert(cel::DestinationContext {
			address,
			port: 443,
			hostname: hostname.map(Into::into),
		});
		request
	}

	#[test]
	fn cidr_rules_authorize_only_matching_destinations() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![protos::ateapi::EgressRule {
				ip_blocks: Some(protos::ateapi::IpBlockRule {
					cidrs: vec!["192.0.2.0/24".to_owned()],
				}),
				..Default::default()
			}],
			..Default::default()
		};
		assert!(matching_rule(&policy, &request("192.0.2.10", None)).is_ok());
		assert!(matching_rule(&policy, &request("198.51.100.10", None)).is_err());
	}

	#[test]
	fn hostname_rules_match_exact_and_single_label_wildcards() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![protos::ateapi::EgressRule {
				hostnames: Some(protos::ateapi::HostnameRule {
					patterns: vec!["api.example.com".to_owned(), "*.example.net".to_owned()],
					..Default::default()
				}),
				..Default::default()
			}],
			..Default::default()
		};
		assert!(matching_rule(&policy, &request("192.0.2.1", Some("api.example.com"))).is_ok());
		assert!(matching_rule(&policy, &request("192.0.2.1", Some("one.example.net"))).is_ok());
		assert!(
			matching_rule(
				&policy,
				&request("192.0.2.1", Some("nested.one.example.net"))
			)
			.is_err()
		);
	}

	#[test]
	fn first_matching_hostname_rule_controls_effects() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![
				protos::ateapi::EgressRule {
					hostnames: Some(protos::ateapi::HostnameRule {
						patterns: vec!["api.example.com".to_owned()],
						effects: Some(protos::ateapi::EgressRuleEffects {
							inject_static_headers: vec![protos::ateapi::CredentialHeaderInjection {
								header: "Authorization".to_owned(),
								prefix: "Bearer ".to_owned(),
								credential_uri: "substrate-secret://example/first/token".to_owned(),
							}],
						}),
					}),
					..Default::default()
				},
				protos::ateapi::EgressRule {
					hostnames: Some(protos::ateapi::HostnameRule {
						patterns: vec!["api.example.com".to_owned()],
						effects: Some(protos::ateapi::EgressRuleEffects {
							inject_static_headers: vec![protos::ateapi::CredentialHeaderInjection {
								header: "Authorization".to_owned(),
								prefix: "Bearer ".to_owned(),
								credential_uri: "substrate-secret://example/second/token".to_owned(),
							}],
						}),
					}),
					..Default::default()
				},
			],
			..Default::default()
		};
		let matched =
			matching_rule(&policy, &request("198.51.100.10", Some("api.example.com"))).unwrap();
		assert_eq!(
			matched
				.hostnames
				.as_ref()
				.unwrap()
				.effects
				.as_ref()
				.unwrap()
				.inject_static_headers[0]
				.credential_uri,
			"substrate-secret://example/first/token"
		);
	}

	#[test]
	fn first_matching_rule_wins_across_matcher_types() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![
				protos::ateapi::EgressRule {
					ip_blocks: Some(protos::ateapi::IpBlockRule {
						cidrs: vec!["192.0.2.0/24".to_owned()],
					}),
					..Default::default()
				},
				protos::ateapi::EgressRule {
					hostnames: Some(protos::ateapi::HostnameRule {
						patterns: vec!["api.example.com".to_owned()],
						..Default::default()
					}),
					..Default::default()
				},
			],
			..Default::default()
		};
		let matched = matching_rule(&policy, &request("192.0.2.10", Some("api.example.com"))).unwrap();
		assert!(matched.ip_blocks.is_some());
	}

	#[test]
	fn all_matches_only_after_earlier_rules_do_not_match() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![
				protos::ateapi::EgressRule {
					hostnames: Some(protos::ateapi::HostnameRule {
						patterns: vec!["api.example.com".to_owned()],
						..Default::default()
					}),
					..Default::default()
				},
				protos::ateapi::EgressRule {
					all: Some(()),
					..Default::default()
				},
			],
			..Default::default()
		};

		let matched =
			matching_rule(&policy, &request("198.51.100.10", Some("api.example.com"))).unwrap();
		assert!(matched.hostnames.is_some());

		let matched = matching_rule(
			&policy,
			&request("198.51.100.10", Some("other.example.com")),
		)
		.unwrap();
		assert!(matched.all.is_some());
	}

	#[test]
	fn no_matching_rule_denies() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![protos::ateapi::EgressRule {
				hostnames: Some(protos::ateapi::HostnameRule {
					patterns: vec!["api.example.com".to_owned()],
					..Default::default()
				}),
				..Default::default()
			}],
			..Default::default()
		};
		assert!(
			matching_rule(
				&policy,
				&request("198.51.100.10", Some("other.example.com"))
			)
			.is_err()
		);
	}

	#[test]
	fn credential_uri_uses_the_exact_authority_as_provider_name() {
		assert_eq!(
			provider_name("substrate-secret://kubernetes.io/default/token"),
			Some("kubernetes.io")
		);
		assert_eq!(provider_name("https://kubernetes.io/default/token"), None);
		assert_eq!(provider_name("substrate-secret:///default/token"), None);
		assert_eq!(
			provider_name("substrate-secret://kubernetes.io:443/token"),
			None
		);
	}

	#[test]
	fn credential_header_overwrites_with_a_sensitive_prefixed_secret() {
		let (name, value) = credential_header(
			&protos::ateapi::CredentialHeaderInjection {
				header: "authorization".to_owned(),
				prefix: "Bearer ".to_owned(),
				credential_uri: "substrate-secret://kubernetes.io/default/token".to_owned(),
			},
			b"token\n".to_vec(),
		)
		.unwrap();
		assert_eq!(name, ::http::header::AUTHORIZATION);
		assert_eq!(value, "Bearer token");
		assert!(value.is_sensitive());
	}

	#[test]
	fn credential_headers_cannot_modify_routing_or_framing_headers() {
		for header in [
			"host",
			"content-length",
			"connection",
			"keep-alive",
			"proxy-authenticate",
			"proxy-authorization",
			"te",
			"trailer",
			"transfer-encoding",
			"upgrade",
		] {
			let injection = protos::ateapi::CredentialHeaderInjection {
				header: header.to_owned(),
				prefix: String::new(),
				credential_uri: "substrate-secret://kubernetes.io/default/token".to_owned(),
			};
			assert!(credential_header(&injection, b"token".to_vec()).is_err());
		}
	}

	#[test]
	fn credential_provider_errors_preserve_availability_semantics() {
		for code in [Code::Unavailable, Code::DeadlineExceeded] {
			let response = credential_provider_error(
				"substrate-secret://kubernetes.io/default/token",
				tonic::Status::new(code, "provider failed"),
			)
			.into_response_with_grpc(false);
			assert_eq!(response.status(), ::http::StatusCode::SERVICE_UNAVAILABLE);
		}

		let response = credential_provider_error(
			"substrate-secret://kubernetes.io/default/token",
			tonic::Status::permission_denied("not allowed"),
		)
		.into_response_with_grpc(false);
		assert_eq!(response.status(), ::http::StatusCode::FORBIDDEN);
	}

	#[test]
	fn malformed_credential_secrets_fail_closed() {
		let injection = protos::ateapi::CredentialHeaderInjection {
			header: "authorization".to_owned(),
			prefix: "Bearer ".to_owned(),
			credential_uri: "substrate-secret://kubernetes.io/default/token".to_owned(),
		};
		assert!(credential_header(&injection, Vec::new()).is_err());
		assert!(credential_header(&injection, b"bad\nsecret".to_vec()).is_err());
	}

	#[test]
	fn credential_cache_reuses_fresh_entries_and_expires_stale_ones() {
		let cache = CredentialCache::new(16);
		let key = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/atespace/default/actor/example".to_owned(),
			uri: "substrate-secret://kubernetes.io/default/token".to_owned(),
		};
		let now = Instant::now();
		cache.insert(key.clone(), b"token".to_vec(), now);
		assert_eq!(
			cache.get(&key, now, DEFAULT_CREDENTIAL_CACHE_TTL),
			Some(b"token".to_vec())
		);

		let stale_at = now - DEFAULT_CREDENTIAL_CACHE_TTL - Duration::from_secs(1);
		cache.insert(key.clone(), b"stale".to_vec(), stale_at);
		assert_eq!(cache.get(&key, now, DEFAULT_CREDENTIAL_CACHE_TTL), None);
	}

	#[test]
	fn credential_cache_is_partitioned_by_actor_identity_and_uri() {
		let cache = CredentialCache::new(16);
		let now = Instant::now();
		let key = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/atespace/default/actor/one".to_owned(),
			uri: "substrate-secret://kubernetes.io/default/token".to_owned(),
		};
		cache.insert(key.clone(), b"one".to_vec(), now);

		let another_actor = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/atespace/default/actor/two".to_owned(),
			uri: key.uri.clone(),
		};
		let another_uri = CredentialCacheKey {
			actor_identity: key.actor_identity.clone(),
			uri: "substrate-secret://kubernetes.io/default/other".to_owned(),
		};
		assert_eq!(
			cache.get(&another_actor, now, DEFAULT_CREDENTIAL_CACHE_TTL),
			None
		);
		assert_eq!(
			cache.get(&another_uri, now, DEFAULT_CREDENTIAL_CACHE_TTL),
			None
		);
	}

	#[test]
	fn credential_providers_accept_inline_backends_with_policies() {
		let provider: CredentialProvider = serde_json::from_value(serde_json::json!({
			"uriAuthority": "kubernetes.io",
			"target": {
				"host": "https://credprovider.example.test:50051",
				"policies": { "backendTLS": {} }
			}
		}))
		.unwrap();
		assert_eq!(provider.uri_authority, "kubernetes.io");
		assert!(matches!(
			provider.target.target.as_ref(),
			crate::types::agent::SimpleBackendReference::InlineBackend(_)
		));
	}
}
