use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::http::{HeaderName, HeaderValue};
use quick_cache::sync::Cache;
use tonic::Code;

use super::{ActorIdentity, EgressActorResolution, TRACE_POLICY_KIND};
use crate::http::{PolicyResponse, Request};
use crate::proxy::httpproxy::{DynamicBackendOverride, PolicyClient};
use crate::proxy::{ProxyError, ProxyResponse};
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::transport::stream::{Extension, TCPConnectionInfo};
use crate::types::agent::{SimpleBackendReferenceWithPolicies, Target};
use crate::{cel, *};

const DEFAULT_CREDENTIAL_CACHE_CAPACITY: usize = 8192;
const DEFAULT_CREDENTIAL_CACHE_TTL: Duration = Duration::from_secs(300);

/// The inner HTTP transport, independent of the actor's CONNECT transport or URI scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EgressRequestProtocol {
	Http,
	Https,
}

/// Retrieves and enforces the current Substrate egress policy for each request.
#[apply(schema!)]
pub struct SubstrateEgress {
	/// Backend that receives GetActorEgressPolicy calls and policies used when connecting to it.
	#[serde(flatten)]
	pub target: SimpleBackendReferenceWithPolicies,
	/// Credential providers available to secret-backed egress effects, keyed by
	/// the authority in an `ate-secret://` URI.
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
		log.ate_actor_name = Some(identity.actor_name.clone());
		log.ate_actor_uid = identity.actor_uid.clone();
		log.ate_atespace = Some(identity.atespace.clone());
		if req.method() == ::http::Method::CONNECT
			|| req.headers().contains_key(::http::header::UPGRADE)
		{
			return Err(
				ProxyError::SubstrateEgressDenied(
					"HTTP upgrades, including CONNECT, are denied for actor egress".to_owned(),
				)
				.into(),
			);
		}
		let policy = fetch_policy(&self.target, client, &identity).await?;
		let matched_rule = matching_rule(&policy, req)?;
		// Dynamic forwarding uses the authorized name and the original destination
		// port. A port supplied in the HTTP authority must not change the dial target.
		let destination = req
			.extensions()
			.get::<cel::DestinationContext>()
			.expect("validated destination");
		let hostname = destination.hostname.as_deref().expect("validated hostname");
		let hostname = hostname
			.strip_prefix('[')
			.and_then(|host| host.strip_suffix(']'))
			.unwrap_or(hostname);
		let target = Target::from((hostname, destination.port));
		req.extensions_mut().insert(DynamicBackendOverride(target));
		self
			.apply_effects(
				client,
				&identity,
				matched_rule
					.http
					.as_ref()
					.and_then(|rule| rule.effects.as_ref())
					.or_else(|| {
						matched_rule
							.https
							.as_ref()
							.and_then(|rule| rule.effects.as_ref())
					}),
				req,
			)
			.await?;
		Ok(PolicyResponse::default())
	}
}

impl SubstrateEgress {
	async fn apply_effects(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
		effects: Option<&protos::ateapi::HttpRuleEffects>,
		req: &mut Request,
	) -> Result<(), ProxyResponse> {
		let Some(effects) = effects else {
			return Ok(());
		};
		for injection in &effects.replace_headers {
			if !req.headers().contains_key(&injection.header) {
				continue;
			}
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

		let policy_client =
			client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate);
		let channel = provider.target.grpc_channel(policy_client.clone());
		let mut request = tonic::Request::new(protos::credprovider::FetchSecretRequest {
			uri: uri.to_owned(),
			actor_spiffe_id: actor_identity,
		});
		let mut span = policy_client.start_grpc_span(
			&mut request,
			provider.target.target.as_ref(),
			"/credprovider.CredentialProvider/FetchSecret",
		);
		let mut provider =
			protos::credprovider::credential_provider_client::CredentialProviderClient::new(channel);
		let response = provider.fetch_secret(request).await;
		if let Some(span) = span.as_deref_mut() {
			span.record_grpc_result(&response);
		}
		drop(span);
		let response = response
			.map_err(|status| credential_provider_error(uri, status))?
			.into_inner();
		let secret = credential_secret(response.opaque_bytes)?;
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
	let authority = uri.strip_prefix("ate-secret://")?.split('/').next()?;
	(!authority.is_empty() && !authority.contains(['?', '#', '@', ':'])).then_some(authority)
}

fn actor_spiffe_uri(atespace: &str, actor_name: &str) -> String {
	// Translate the authenticated ateom identity to the actor identity providers authorize.
	// Matches Substrate's resources.ActorSPIFFEID.
	format!("spiffe://substrate-actor.local/actor/{atespace}/{actor_name}")
}

fn credential_header(
	injection: &protos::ateapi::CredentialHeader,
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

async fn fetch_policy(
	target: &SimpleBackendReferenceWithPolicies,
	client: &PolicyClient,
	identity: &ActorIdentity,
) -> Result<protos::ateapi::EgressPolicy, ProxyError> {
	let policy_client =
		client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate);
	let channel = target.grpc_channel(policy_client.clone());
	let mut control = protos::ateapi::control_client::ControlClient::new(channel);
	let mut request = tonic::Request::new(protos::ateapi::GetActorEgressPolicyRequest {
		actor: Some(protos::ateapi::ObjectRef {
			atespace: identity.atespace.clone(),
			name: identity.actor_name.clone(),
		}),
	});
	let mut span = policy_client.start_grpc_span(
		&mut request,
		target.target.as_ref(),
		"/ateapi.Control/GetActorEgressPolicy",
	);
	let response = crate::proxy::dtrace::scope_future(
		Some(TRACE_POLICY_KIND),
		control.get_actor_egress_policy(request),
	)
	.await;
	if let Some(span) = span.as_deref_mut() {
		span.record_grpc_result(&response);
	}
	drop(span);
	match response {
		Ok(response) => Ok(response.into_inner()),
		Err(status) if matches!(status.code(), Code::Unavailable | Code::DeadlineExceeded) => Err(
			ProxyError::SubstrateEgressUnavailable(format!("actor egress policy unavailable: {status}")),
		),
		Err(status) => Err(ProxyError::SubstrateEgressDenied(format!(
			"actor egress policy denied: {status}"
		))),
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EgressTlsMode {
	Intercept,
	// Complete TLS, then return 403 before routing or applying request policies.
	InterceptDenied,
	Passthrough,
}

pub(crate) async fn authorize_tls(
	client: &PolicyClient,
	connection: &mut Extension,
	sni: &str,
) -> Result<Option<EgressTlsMode>, ProxyError> {
	let Some(identity) = connection.get::<ActorIdentity>() else {
		return Ok(None);
	};
	let policy = connection.get::<EgressActorResolution>().ok_or_else(|| {
		ProxyError::SubstrateEgressDenied("missing actor egress control API".to_owned())
	})?;
	let port = connection
		.get::<TCPConnectionInfo>()
		.expect("tcp connection must be set")
		.local_addr
		.port();
	let policy = fetch_policy(&policy.target, client, identity).await?;
	let mode = tls_mode(&policy, sni, port)?;
	if mode == EgressTlsMode::Passthrough {
		connection.insert(DynamicBackendOverride(Target::from((sni, port))));
	}
	Ok(Some(mode))
}

fn tls_mode(
	policy: &protos::ateapi::EgressPolicy,
	sni: &str,
	port: u16,
) -> Result<EgressTlsMode, ProxyError> {
	let sni = sni.strip_suffix('.').unwrap_or(sni).to_ascii_lowercase();
	if !valid_hostname(&sni) {
		return Err(ProxyError::SubstrateEgressDenied(
			"missing or invalid TLS SNI".to_owned(),
		));
	}
	let Some(rule) = matching_destination_rule(policy, &sni, port, MatchPhase::ClientHello) else {
		return Ok(EgressTlsMode::InterceptDenied);
	};
	Ok(if rule.https.is_some() {
		EgressTlsMode::Intercept
	} else {
		EgressTlsMode::Passthrough
	})
}

#[derive(Clone, Copy)]
enum MatchPhase {
	HttpRequest,
	HttpsRequest,
	ClientHello,
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
	let protocol = req
		.extensions()
		.get::<EgressRequestProtocol>()
		.ok_or_else(|| {
			ProxyError::SubstrateEgressDenied("missing egress protocol context".to_owned())
		})?;
	let phase = match protocol {
		EgressRequestProtocol::Http => MatchPhase::HttpRequest,
		EgressRequestProtocol::Https => MatchPhase::HttpsRequest,
	};
	matching_destination_rule(
		policy,
		destination.hostname.as_deref().unwrap_or_default(),
		destination.port,
		phase,
	)
	.ok_or_else(|| {
		ProxyError::SubstrateEgressDenied("actor egress policy denied destination".to_owned()).into()
	})
}

fn matching_destination_rule<'a>(
	policy: &'a protos::ateapi::EgressPolicy,
	hostname: &str,
	port: u16,
	phase: MatchPhase,
) -> Option<&'a protos::ateapi::EgressRule> {
	let mut best = None;
	for rule in &policy.rules {
		// These fields form an API union. An invalid union cannot authorize traffic.
		let (hostnames, ports, default_port) =
			match (phase, &rule.http, &rule.https, &rule.tls_passthrough) {
				(MatchPhase::HttpRequest, Some(http), None, None) => {
					(&http.hostnames, http.ports.as_ref(), 80)
				},
				(MatchPhase::HttpsRequest | MatchPhase::ClientHello, None, Some(https), None) => {
					(&https.hostnames, https.ports.as_ref(), 443)
				},
				(MatchPhase::ClientHello, None, None, Some(tls)) => (&tls.hostnames, tls.ports.as_ref(), 0),
				_ => continue,
			};
		let Some(port_rank) = port_rank(ports, port, default_port) else {
			continue;
		};
		let Some(name_rank) = hostnames
			.iter()
			.filter_map(|pattern| hostname_rank(pattern, hostname))
			.min()
		else {
			continue;
		};
		let rank = (name_rank, port_rank);
		if best.is_none_or(|(_, best_rank)| rank < best_rank) {
			best = Some((rule, rank));
		}
	}
	best.map(|(rule, _)| rule)
}

fn port_rank(ports: Option<&protos::ateapi::Ports>, port: u16, default_port: u16) -> Option<u8> {
	if port == 0 {
		return None;
	}
	let Some(ports) = ports else {
		return (port == default_port).then_some(0);
	};
	if ports.all.is_some() {
		return ports.numbers.is_empty().then_some(1);
	}
	if ports
		.numbers
		.iter()
		.any(|number| !(1..=65535).contains(number))
	{
		return None;
	}
	ports.numbers.contains(&i32::from(port)).then_some(0)
}

fn hostname_rank(pattern: &str, hostname: &str) -> Option<u8> {
	if pattern == "*" {
		let host = hostname
			.strip_prefix('[')
			.and_then(|host| host.strip_suffix(']'))
			.unwrap_or(hostname);
		return (valid_hostname(hostname) || host.parse::<std::net::IpAddr>().is_ok()).then_some(2);
	}
	if !valid_hostname(hostname) {
		return None;
	}
	if let Some(suffix) = pattern.strip_prefix("*.") {
		if !valid_hostname(suffix) {
			return None;
		}
		let label = hostname.strip_suffix(suffix)?.strip_suffix('.')?;
		(!label.is_empty() && !label.contains('.')).then_some(1)
	} else {
		(pattern == hostname).then_some(0)
	}
}

fn valid_hostname(hostname: &str) -> bool {
	hostname.len() <= 253
		&& hostname.split('.').all(super::valid_resource_name)
		&& !hostname
			.rsplit('.')
			.next()
			.unwrap_or_default()
			.bytes()
			.all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
	use std::net::IpAddr;

	use super::*;

	fn request(protocol: EgressRequestProtocol, hostname: &str, port: u16) -> Request {
		let mut request = Request::new(crate::http::Body::empty());
		request.extensions_mut().insert(cel::DestinationContext {
			address: "192.0.2.1".parse::<IpAddr>().unwrap(),
			port,
			hostname: Some(hostname.into()),
		});
		request.extensions_mut().insert(protocol);
		request
	}

	fn rule(
		protocol: EgressRequestProtocol,
		hostnames: &[&str],
		ports: Option<protos::ateapi::Ports>,
	) -> protos::ateapi::EgressRule {
		let hostnames = hostnames.iter().map(|name| (*name).to_owned()).collect();
		match protocol {
			EgressRequestProtocol::Http => protos::ateapi::EgressRule {
				http: Some(protos::ateapi::HttpRule {
					hostnames,
					ports,
					effects: None,
				}),
				..Default::default()
			},
			EgressRequestProtocol::Https => protos::ateapi::EgressRule {
				https: Some(protos::ateapi::HttpsRule {
					hostnames,
					ports,
					effects: None,
				}),
				..Default::default()
			},
		}
	}

	fn all_ports() -> Option<protos::ateapi::Ports> {
		Some(protos::ateapi::Ports {
			all: Some(protos::ateapi::AllPorts {}),
			numbers: vec![],
		})
	}

	fn ports(numbers: &[i32]) -> Option<protos::ateapi::Ports> {
		Some(protos::ateapi::Ports {
			all: None,
			numbers: numbers.to_vec(),
		})
	}

	#[test]
	fn rules_are_protocol_specific_and_default_ports_are_enforced() {
		for (protocol, other, default_port) in [
			(
				EgressRequestProtocol::Http,
				EgressRequestProtocol::Https,
				80,
			),
			(
				EgressRequestProtocol::Https,
				EgressRequestProtocol::Http,
				443,
			),
		] {
			let policy = protos::ateapi::EgressPolicy {
				rules: vec![rule(protocol, &["api.example.com"], None)],
				..Default::default()
			};
			assert!(matching_rule(&policy, &request(protocol, "api.example.com", default_port)).is_ok());
			assert!(matching_rule(&policy, &request(other, "api.example.com", default_port)).is_err());
			assert!(matching_rule(&policy, &request(protocol, "api.example.com", 8443)).is_err());
			assert!(
				matching_rule(
					&policy,
					&request(protocol, "other.example.com", default_port)
				)
				.is_err()
			);
		}
	}

	#[test]
	fn explicit_and_all_ports() {
		for (selected_ports, port, allowed) in [
			(ports(&[80, 8080]), 8080, true),
			(ports(&[80, 8080]), 443, false),
			(all_ports(), 8443, true),
			(all_ports(), 0, false),
			(ports(&[]), 80, false),
			(ports(&[-1, 80]), 80, false),
			(ports(&[65536, 80]), 80, false),
			(
				Some(protos::ateapi::Ports {
					all: Some(protos::ateapi::AllPorts {}),
					numbers: vec![80],
				}),
				80,
				false,
			),
		] {
			let policy = protos::ateapi::EgressPolicy {
				rules: vec![rule(EgressRequestProtocol::Http, &["*"], selected_ports)],
				..Default::default()
			};
			assert_eq!(
				matching_rule(
					&policy,
					&request(EgressRequestProtocol::Http, "api.example.com", port)
				)
				.is_ok(),
				allowed
			);
		}
	}

	#[test]
	fn hostnames_match_exact_single_label_and_any_patterns() {
		for (pattern, hostname, expected) in [
			("api.example.com", "api.example.com", Some(0)),
			("api.example.com", "other.example.com", None),
			("*.example.com", "api.example.com", Some(1)),
			("*.example.com", "nested.api.example.com", None),
			("*.example.com", "example.com", None),
			("*.example.com", ".example.com", None),
			("*", "api.example.com", Some(2)),
			("*", "192.0.2.1", Some(2)),
			("*", "[2001:db8::1]", Some(2)),
			("192.0.2.1", "192.0.2.1", None),
			("*.2.1", "192.0.2.1", None),
			("*", "", None),
			("*", "foo..example.com", None),
			("*", "example.com.", None),
			("*", "example.com:80", None),
			("*", "-invalid.example", None),
			("*", "01.2.3.4", None),
			("*", "unicode.Kom", None),
			("*", "user@example.com", None),
		] {
			assert_eq!(
				hostname_rank(pattern, hostname),
				expected,
				"{pattern}: {hostname}"
			);
		}
	}

	#[test]
	fn most_specific_rule_wins_independent_of_order() {
		let protocol = EgressRequestProtocol::Https;
		// Name specificity precedes port specificity. Match the best pattern in each rule.
		let ranked = [
			rule(
				protocol,
				&["unrelated.example", "api.example.com"],
				ports(&[443]),
			),
			rule(protocol, &["api.example.com"], all_ports()),
			rule(protocol, &["*.example.com"], ports(&[443])),
			rule(protocol, &["*.example.com"], all_ports()),
			rule(protocol, &["*"], ports(&[443])),
			rule(protocol, &["*"], all_ports()),
		];
		for pair in ranked.windows(2) {
			for rules in [
				vec![pair[0].clone(), pair[1].clone()],
				vec![pair[1].clone(), pair[0].clone()],
			] {
				let policy = protos::ateapi::EgressPolicy {
					rules,
					..Default::default()
				};
				assert_eq!(
					matching_rule(&policy, &request(protocol, "api.example.com", 443)).unwrap(),
					&pair[0]
				);
			}
		}
	}

	#[test]
	fn passthrough_empty_and_invalid_rules_cannot_authorize_http_requests() {
		let passthrough = protos::ateapi::EgressRule {
			tls_passthrough: Some(protos::ateapi::TlsPassthroughRule {
				hostnames: vec!["*".to_owned()],
				ports: all_ports(),
			}),
			..Default::default()
		};
		let mut invalid = rule(EgressRequestProtocol::Http, &["*"], all_ports());
		invalid.https = rule(EgressRequestProtocol::Https, &["*"], all_ports()).https;
		for rules in [
			vec![],
			vec![Default::default()],
			vec![passthrough],
			vec![invalid],
		] {
			let policy = protos::ateapi::EgressPolicy {
				rules,
				..Default::default()
			};
			for protocol in [EgressRequestProtocol::Http, EgressRequestProtocol::Https] {
				assert!(matching_rule(&policy, &request(protocol, "api.example.com", 443)).is_err());
			}
		}
	}

	#[test]
	fn missing_transport_or_destination_denies() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![rule(EgressRequestProtocol::Http, &["*"], all_ports())],
			..Default::default()
		};
		let mut req = request(EgressRequestProtocol::Http, "api.example.com", 80);
		req.extensions_mut().remove::<EgressRequestProtocol>();
		assert!(matching_rule(&policy, &req).is_err());
		req.extensions_mut().insert(EgressRequestProtocol::Http);
		req.extensions_mut().remove::<cel::DestinationContext>();
		assert!(matching_rule(&policy, &req).is_err());
	}

	fn passthrough(
		hostnames: &[&str],
		ports: Option<protos::ateapi::Ports>,
	) -> protos::ateapi::EgressRule {
		protos::ateapi::EgressRule {
			tls_passthrough: Some(protos::ateapi::TlsPassthroughRule {
				hostnames: hostnames.iter().map(|name| (*name).to_owned()).collect(),
				ports,
			}),
			..Default::default()
		}
	}

	#[test]
	fn tls_selection_compares_https_and_passthrough_specificity() {
		for (https, tls, expected) in [
			(
				rule(
					EgressRequestProtocol::Https,
					&["api.example.com"],
					all_ports(),
				),
				passthrough(&["*.example.com"], ports(&[443])),
				EgressTlsMode::Intercept,
			),
			(
				rule(
					EgressRequestProtocol::Https,
					&["*.example.com"],
					ports(&[443]),
				),
				passthrough(&["api.example.com"], all_ports()),
				EgressTlsMode::Passthrough,
			),
			(
				rule(EgressRequestProtocol::Https, &["api.example.com"], None),
				passthrough(&["api.example.com"], all_ports()),
				EgressTlsMode::Intercept,
			),
			(
				rule(
					EgressRequestProtocol::Https,
					&["api.example.com"],
					all_ports(),
				),
				passthrough(&["api.example.com"], ports(&[443])),
				EgressTlsMode::Passthrough,
			),
		] {
			for rules in [vec![https.clone(), tls.clone()], vec![tls, https]] {
				let policy = protos::ateapi::EgressPolicy {
					rules,
					..Default::default()
				};
				assert_eq!(
					tls_mode(&policy, "API.Example.com.", 443).unwrap(),
					expected
				);
			}
		}
	}

	#[test]
	fn tls_rejects_invalid_sni_and_intercepts_unmatched_destinations_for_denial() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![passthrough(&["*"], ports(&[443]))],
			..Default::default()
		};
		for sni in ["", "127.0.0.1", "bad..example", "example.com.."] {
			assert!(tls_mode(&policy, sni, 443).is_err(), "{sni}");
		}
		assert_eq!(
			tls_mode(&policy, "api.example.com", 8443).unwrap(),
			EgressTlsMode::InterceptDenied
		);
		for rules in [
			vec![],
			vec![passthrough(&["*"], None)],
			vec![rule(EgressRequestProtocol::Http, &["*"], all_ports())],
		] {
			let policy = protos::ateapi::EgressPolicy {
				rules,
				..Default::default()
			};
			assert_eq!(
				tls_mode(&policy, "api.example.com", 443).unwrap(),
				EgressTlsMode::InterceptDenied
			);
		}
	}

	#[test]
	fn credential_uri_uses_the_exact_authority_as_provider_name() {
		assert_eq!(
			provider_name("ate-secret://kubernetes.io/default/token"),
			Some("kubernetes.io")
		);
		assert_eq!(provider_name("https://kubernetes.io/default/token"), None);
		assert_eq!(
			provider_name("substrate-secret://kubernetes.io/default/token"),
			None
		);
		assert_eq!(provider_name("ate-secret:///default/token"), None);
		assert_eq!(provider_name("ate-secret://kubernetes.io:443/token"), None);
	}

	#[test]
	fn credential_header_overwrites_with_a_sensitive_prefixed_secret() {
		let (name, value) = credential_header(
			&protos::ateapi::CredentialHeader {
				header: "authorization".to_owned(),
				prefix: "Bearer ".to_owned(),
				credential_uri: "ate-secret://kubernetes.io/default/token".to_owned(),
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
			let injection = protos::ateapi::CredentialHeader {
				header: header.to_owned(),
				prefix: String::new(),
				credential_uri: "ate-secret://kubernetes.io/default/token".to_owned(),
			};
			assert!(credential_header(&injection, b"token".to_vec()).is_err());
		}
	}

	#[test]
	fn credential_provider_errors_preserve_availability_semantics() {
		for code in [Code::Unavailable, Code::DeadlineExceeded] {
			let response = credential_provider_error(
				"ate-secret://kubernetes.io/default/token",
				tonic::Status::new(code, "provider failed"),
			)
			.into_response_with_grpc(false);
			assert_eq!(response.status(), ::http::StatusCode::SERVICE_UNAVAILABLE);
		}

		let response = credential_provider_error(
			"ate-secret://kubernetes.io/default/token",
			tonic::Status::permission_denied("not allowed"),
		)
		.into_response_with_grpc(false);
		assert_eq!(response.status(), ::http::StatusCode::FORBIDDEN);
	}

	#[test]
	fn malformed_credential_secrets_fail_closed() {
		let injection = protos::ateapi::CredentialHeader {
			header: "authorization".to_owned(),
			prefix: "Bearer ".to_owned(),
			credential_uri: "ate-secret://kubernetes.io/default/token".to_owned(),
		};
		assert!(credential_header(&injection, Vec::new()).is_err());
		assert!(credential_header(&injection, b"bad\nsecret".to_vec()).is_err());
	}

	#[test]
	fn credential_cache_reuses_fresh_entries_and_expires_stale_ones() {
		let cache = CredentialCache::new(16);
		let key = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/actor/default/example".to_owned(),
			uri: "ate-secret://kubernetes.io/default/token".to_owned(),
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
			actor_identity: "spiffe://substrate-actor.local/actor/default/one".to_owned(),
			uri: "ate-secret://kubernetes.io/default/token".to_owned(),
		};
		cache.insert(key.clone(), b"one".to_vec(), now);

		let another_actor = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/actor/default/two".to_owned(),
			uri: key.uri.clone(),
		};
		let another_uri = CredentialCacheKey {
			actor_identity: key.actor_identity.clone(),
			uri: "ate-secret://kubernetes.io/default/other".to_owned(),
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
