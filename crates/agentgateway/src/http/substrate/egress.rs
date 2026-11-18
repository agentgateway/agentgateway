use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::http::{HeaderName, HeaderValue};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use quick_cache::sync::Cache;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tonic::Code;
use x509_parser::extensions::{GeneralName, ParsedExtension};
use x509_parser::prelude::FromDer;

use super::{ActorIdentity, EgressActorResolution, TRACE_POLICY_KIND};
use crate::http::backendtls::ClientCertificate;
use crate::http::{PolicyResponse, Request};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::{DynamicBackendOverride, PolicyClient};
use crate::store::RequestPolicyTrait;
use crate::telemetry::log::RequestLog;
use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
use crate::transport::stream::{Extension, TCPConnectionInfo};
use crate::types::agent::{SimpleBackendReferenceWithPolicies, Target};
use crate::{cel, *};

const DEFAULT_CREDENTIAL_CACHE_CAPACITY: usize = 8192;
const DEFAULT_CREDENTIAL_CACHE_TTL: Duration = Duration::from_secs(300);
const INVALID_CERTIFICATE_CACHE_TTL: Duration = Duration::from_secs(5);
const CREDENTIAL_FETCH_TIMEOUT: Duration = Duration::from_secs(4);

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
	#[serde(skip, default = "default_credential_cache")]
	#[cfg_attr(feature = "schema", schemars(skip))]
	certificate_cache: CredentialCache<Result<ClientCertificate, String>>,
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
	/// How long to reuse a fetched credential. Defaults to 5 minutes; `0s` disables caching.
	#[serde(
		default,
		with = "crate::serdes::serde_dur_option",
		skip_serializing_if = "Option::is_none"
	)]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub cache_ttl: Option<Duration>,
}

#[derive(Debug, Clone)]
struct CredentialCache<T = Vec<u8>> {
	entries: Arc<Cache<CredentialCacheKey, CachedCredential<T>>>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct CredentialCacheKey {
	actor_identity: String,
	actor_uid: Option<String>,
	uri: String,
}

#[derive(Clone)]
struct CachedCredential<T> {
	secret: T,
	fetched_at: Instant,
	ttl: Duration,
}

impl<T: Clone> CredentialCache<T> {
	fn new(capacity: usize) -> Self {
		Self {
			entries: Arc::new(Cache::new(capacity)),
		}
	}

	fn get(&self, key: &CredentialCacheKey, now: Instant) -> Option<T> {
		if let Some(entry) = self.entries.get(key) {
			if now.duration_since(entry.fetched_at) < entry.ttl {
				return Some(entry.secret);
			}
			self.entries.remove_if(key, |entry| {
				now.duration_since(entry.fetched_at) >= entry.ttl
			});
		}
		None
	}

	fn insert(&self, key: CredentialCacheKey, secret: T, now: Instant, ttl: Duration) {
		if !ttl.is_zero() {
			self.entries.insert(
				key,
				CachedCredential {
					secret,
					fetched_at: now,
					ttl,
				},
			);
		}
	}
}

fn default_credential_cache<T: Clone>() -> CredentialCache<T> {
	CredentialCache::new(DEFAULT_CREDENTIAL_CACHE_CAPACITY)
}

impl RequestPolicyTrait for SubstrateEgress {
	async fn apply(
		&self,
		client: &PolicyClient,
		log: &mut RequestLog,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyError> {
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
		let binding = if let Some(source) = matched_rule
			.https
			.as_ref()
			.and_then(|rule| rule.upstream_tls.as_ref())
			.and_then(|tls| tls.client_certificate.as_ref())
		{
			let certificate = self.client_certificate(client, &identity, source).await?;
			let thumbprint = certificate.leaf_sha256();
			req.extensions_mut().insert(certificate);
			Some(thumbprint)
		} else {
			None
		};
		self
			.apply_effects(
				client,
				&identity,
				binding,
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

pub(crate) fn reject_upgrade(req: &Request) -> Result<(), ProxyError> {
	if req.method() != ::http::Method::CONNECT && !req.headers().contains_key(::http::header::UPGRADE)
	{
		return Ok(());
	}
	let websocket = req
		.headers()
		.get_all(::http::header::UPGRADE)
		.iter()
		.filter_map(|value| value.to_str().ok())
		.flat_map(|value| value.split(','))
		.any(|token| token.trim().eq_ignore_ascii_case("websocket"));
	Err(ProxyError::SubstrateEgressUpgradeDenied(if websocket {
		"WebSocket egress is not supported"
	} else if req.method() == ::http::Method::CONNECT {
		"CONNECT egress is not supported"
	} else {
		"HTTP upgrades, including CONNECT, are denied for actor egress"
	}))
}

impl SubstrateEgress {
	async fn apply_effects(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
		binding: Option<[u8; 32]>,
		effects: Option<&protos::ateapi::HttpRuleEffects>,
		req: &mut Request,
	) -> Result<(), ProxyError> {
		let Some(effects) = effects else {
			return Ok(());
		};
		for injection in &effects.replace_headers {
			if !req.headers().contains_key(&injection.header) {
				continue;
			}
			let secret = match (injection.credential_uri.as_str(), &injection.actor_jwt) {
				(uri, None) if !uri.is_empty() => {
					let provider = self.provider_for_uri(uri)?;
					self.credential(client, identity, provider, uri).await?
				},
				("", Some(source)) => self.actor_jwt(client, identity, source, binding).await?,
				_ => {
					return Err(ProxyError::SubstrateEgressDenied(
						"credential effect must specify exactly one credential source".to_owned(),
					));
				},
			};
			let (name, value) = credential_header(injection, secret)?;
			req.headers_mut().insert(name, value);
		}
		Ok(())
	}

	async fn actor_jwt(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
		source: &protos::ateapi::ActorJwtSource,
		binding: Option<[u8; 32]>,
	) -> Result<Vec<u8>, ProxyError> {
		let key = if binding.is_none() || identity.actor_uid.is_some() {
			let mut audiences = source.audiences.clone();
			audiences.sort_unstable();
			let thumbprint = binding
				.map(|sha256| URL_SAFE_NO_PAD.encode(sha256))
				.unwrap_or_default();
			Some(CredentialCacheKey {
				actor_identity: actor_spiffe_uri(&identity.atespace, &identity.actor_name),
				actor_uid: identity.actor_uid.clone(),
				uri: format!(
					"actor-jwt:{}:{thumbprint}:{audiences:?}",
					source.expiration_seconds
				),
			})
		} else {
			None
		}; let ttl = Duration::from_secs(
			u64::try_from(source.expiration_seconds)
				.unwrap_or_default()
				.saturating_mul(2)
				/ 3,
		);
		if let Some(jwt) = key
			.as_ref()
			.and_then(|key| self.credential_cache.get(key, Instant::now()))
		{
			return Ok(jwt);
		}

		let policy_client =
			client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate);
		let channel = self.target.grpc_channel(policy_client.clone());
		let mut request = tonic::Request::new(protos::ateapi::MintActorJwtRequest {
			actor: Some(protos::ateapi::ObjectRef {
				atespace: identity.atespace.clone(),
				name: identity.actor_name.clone(),
			}),
			audiences: source.audiences.clone(),
			expiration_seconds: source.expiration_seconds,
			client_certificate_sha256: binding.map_or_else(Vec::new, |sha256| sha256.to_vec()),
		});
		let mut span = policy_client.start_grpc_span(
			&mut request,
			self.target.target.as_ref(),
			"/substrate.ateapi.v1.Control/MintActorJwt",
		);
		let mut control = protos::ateapi::control_client::ControlClient::new(channel);
		let requested_at = Instant::now();
		let response = control.mint_actor_jwt(request).await;
		if let Some(span) = span.as_deref_mut() {
			span.record_grpc_result(&response);
		}
		drop(span);
		let response = response.map_err(actor_jwt_error)?.into_inner();
		let jwt = credential_secret(response.actor_jwt.into_bytes())?;
		if let Some(key) = key {
			self
				.credential_cache
				.insert(key, jwt.clone(), requested_at, ttl);
		}
		Ok(jwt)
	}

	async fn credential(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
		provider: &CredentialProvider,
		uri: &str,
	) -> Result<Vec<u8>, ProxyError> {
		let actor_identity = actor_spiffe_uri(&identity.atespace, &identity.actor_name);
		let key = CredentialCacheKey {
			actor_identity: actor_identity.clone(),
			actor_uid: identity.actor_uid.clone(),
			uri: uri.to_owned(),
		};
		let ttl = provider.cache_ttl.unwrap_or(DEFAULT_CREDENTIAL_CACHE_TTL);
		if let Some(secret) = self.credential_cache.get(&key, Instant::now()) {
			return Ok(secret);
		}

		let secret =
			credential_secret(Self::fetch_secret(client, provider, uri, actor_identity).await?)?;
		self
			.credential_cache
			.insert(key, secret.clone(), Instant::now(), ttl);
		Ok(secret)
	}

	async fn fetch_secret(
		client: &PolicyClient,
		provider: &CredentialProvider,
		uri: &str,
		actor_identity: String,
	) -> Result<Vec<u8>, ProxyError> {
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
			"/substrate.credprovider.v1.CredentialProvider/FetchSecret",
		);
		let mut provider =
			protos::credprovider::credential_provider_client::CredentialProviderClient::new(channel);
		let response = tokio::time::timeout(CREDENTIAL_FETCH_TIMEOUT, provider.fetch_secret(request))
			.await
			.unwrap_or_else(|_| Err(tonic::Status::deadline_exceeded("FetchSecret timed out")));
		if let Some(span) = span.as_deref_mut() {
			span.record_grpc_result(&response);
		}
		drop(span);
		Ok(
			response
				.map_err(|status| credential_provider_error(uri, status))?
				.into_inner()
				.opaque_bytes,
		)
	}

	async fn client_certificate(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
		source: &protos::ateapi::ClientCertificateSource,
	) -> Result<ClientCertificate, ProxyError> {
		use protos::ateapi::client_certificate_source::Source;
		if matches!(&source.source, Some(Source::ActorCertificate(_))) {
			return self.actor_client_certificate(client, identity).await;
		}
		let Some(Source::Credential(source)) = &source.source else {
			return Err(ProxyError::SubstrateEgressDenied(
				"client certificate source is missing".to_owned(),
			));
		};
		let uri = &source.credential_uri;
		let provider = self.provider_for_uri(uri)?;
		let actor_identity = actor_spiffe_uri(&identity.atespace, &identity.actor_name);
		let ttl = provider.cache_ttl.unwrap_or(DEFAULT_CREDENTIAL_CACHE_TTL);
		let key = identity
			.actor_uid
			.as_ref()
			.filter(|_| !ttl.is_zero())
			.map(|uid| CredentialCacheKey {
				actor_identity: actor_identity.clone(),
				actor_uid: Some(uid.clone()),
				uri: uri.clone(),
			});
		if let Some(cached) = key
			.as_ref()
			.and_then(|key| self.certificate_cache.get(key, Instant::now()))
		{
			match cached {
				Ok(cert) if cert.is_valid_now() => return Ok(cert),
				Err(message) => return Err(ProxyError::SubstrateEgressDenied(message)),
				Ok(_) => {},
			}
		}

		let pem = Self::fetch_secret(client, provider, uri, actor_identity).await?;
		let result = ClientCertificate::from_pem_bundle(&pem)
			.map_err(|err| format!("credential provider returned an invalid client certificate: {err}"));
		if let Some(key) = key {
			let ttl = if result.is_ok() {
				ttl
			} else {
				ttl.min(INVALID_CERTIFICATE_CACHE_TTL)
			};
			self
				.certificate_cache
				.insert(key, result.clone(), Instant::now(), ttl);
		}
		result.map_err(ProxyError::SubstrateEgressDenied)
	}

	async fn actor_client_certificate(
		&self,
		client: &PolicyClient,
		identity: &ActorIdentity,
	) -> Result<ClientCertificate, ProxyError> {
		let actor_uid = identity.actor_uid.clone().ok_or_else(|| {
			ProxyError::SubstrateEgressDenied("actor identity is missing a UID".to_owned())
		})?;
		let cache_key = CredentialCacheKey {
			actor_identity: actor_spiffe_uri(&identity.atespace, &identity.actor_name),
			actor_uid: Some(actor_uid.clone()),
			uri: "actor-certificate".into(),
		};
		if let Some(Ok(cert)) = self.certificate_cache.get(&cache_key, Instant::now())
			&& cert.is_valid_now()
		{
			return Ok(cert);
		}

		let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|error| {
			ProxyError::SubstrateEgressUnavailable(format!(
				"could not generate actor client certificate key: {error}"
			))
		})?;
		let csr = rcgen::CertificateParams::default()
			.serialize_request(&key)
			.map_err(|error| {
				ProxyError::SubstrateEgressUnavailable(format!(
					"could not generate actor client certificate request: {error}"
				))
			})?;
		let policy_client =
			client.with_outbound(OutboundCallKind::Policy, OutboundCallSubtype::Substrate);
		let channel = self.target.grpc_channel(policy_client.clone());
		let mut request = tonic::Request::new(protos::ateapi::MintActorCertificateRequest {
			actor: Some(protos::ateapi::ObjectRef {
				atespace: identity.atespace.clone(),
				name: identity.actor_name.clone(),
			}),
			actor_uid: actor_uid.clone(),
			certificate_signing_request: csr.der().to_vec(),
		});
		let mut span = policy_client.start_grpc_span(
			&mut request,
			self.target.target.as_ref(),
			"/substrate.ateapi.v1.Control/MintActorCertificate",
		);
		let mut control = protos::ateapi::control_client::ControlClient::new(channel);
		let response = control.mint_actor_certificate(request).await;
		if let Some(span) = span.as_deref_mut() {
			span.record_grpc_result(&response);
		}
		drop(span);
		let response = response.map_err(actor_certificate_error)?.into_inner();
		let chain = response
			.actor_certificates
			.into_iter()
			.map(CertificateDer::from)
			.collect::<Vec<_>>();
		validate_actor_client_certificate(&chain, identity, &actor_uid)?;
		let certificate =
			ClientCertificate::from_der(chain, PrivateKeyDer::Pkcs8(key.serialize_der().into()))
				.map_err(|error| {
					ProxyError::SubstrateEgressDenied(format!(
						"actor certificate issuer returned an invalid client certificate: {error}"
					))
				})?;
		let now = Instant::now();
		let validity = certificate.validity();
		let not_before = validity.not_before.timestamp();
		let lifetime = validity.not_after.timestamp() - not_before;
		let refresh_at = not_before + lifetime * 2 / 3;
		let remaining = refresh_at - x509_parser::time::ASN1Time::now().timestamp();
		self.certificate_cache.insert(
			cache_key,
			Ok(certificate.clone()),
			now,
			Duration::from_secs(u64::try_from(remaining).unwrap_or_default()),
		);
		Ok(certificate)
	}

	fn provider_for_uri(&self, uri: &str) -> Result<&CredentialProvider, ProxyError> {
		let name = provider_name(uri)
			.ok_or_else(|| ProxyError::SubstrateEgressDenied(format!("invalid credential URI: {uri}")))?;
		self
			.credential_providers
			.iter()
			.find(|provider| provider.uri_authority == name)
			.ok_or_else(|| {
				ProxyError::SubstrateEgressDenied(format!("no credential provider configured for {name}"))
			})
	}
}

fn provider_name(uri: &str) -> Option<&str> {
	let authority = uri.strip_prefix("ate-secret://")?.split('/').next()?;
	(!authority.is_empty() && !authority.contains(['?', '#', '@', ':'])).then_some(authority)
}

fn actor_spiffe_uri(atespace: &str, actor_name: &str) -> String {
	// Translate the authenticated worker identity to the actor identity providers authorize.
	// Matches Substrate's resources.ActorSPIFFEID.
	format!("spiffe://substrate-actor.local/actor/{atespace}/{actor_name}")
}

fn credential_header(
	injection: &protos::ateapi::CredentialHeader,
	secret: Vec<u8>,
) -> Result<(HeaderName, HeaderValue), ProxyError> {
	let name = HeaderName::from_str(&injection.header).map_err(|error| {
		ProxyError::SubstrateEgressDenied(format!("invalid credential header: {error}"))
	})?;
	if protected_credential_header(&name) {
		return Err(ProxyError::SubstrateEgressDenied(format!(
			"credential effects cannot modify protected header {name}"
		)));
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

fn actor_jwt_error(status: tonic::Status) -> ProxyError {
	match status.code() {
		Code::Unavailable | Code::DeadlineExceeded => {
			ProxyError::SubstrateEgressUnavailable(format!("actor JWT issuer unavailable: {status}"))
		},
		_ => ProxyError::SubstrateEgressDenied(format!("actor JWT denied: {status}")),
	}
}

fn actor_certificate_error(status: tonic::Status) -> ProxyError {
	match status.code() {
		Code::Unavailable | Code::DeadlineExceeded => ProxyError::SubstrateEgressUnavailable(format!(
			"actor certificate issuer unavailable: {status}"
		)),
		_ => ProxyError::SubstrateEgressDenied(format!("actor certificate issuer denied: {status}")),
	}
}

fn validate_actor_client_certificate(
	chain: &[CertificateDer<'static>],
	identity: &ActorIdentity,
	actor_uid: &str,
) -> Result<(), ProxyError> {
	let invalid = |message: &str| ProxyError::SubstrateEgressDenied(message.to_owned());
	let leaf = chain
		.first()
		.ok_or_else(|| invalid("actor certificate chain is empty"))?;
	let (_, leaf) = x509_parser::certificate::X509Certificate::from_der(leaf.as_ref())
		.map_err(|_| invalid("actor certificate leaf is invalid"))?;
	let expected_spiffe = actor_spiffe_uri(&identity.atespace, &identity.actor_name);
	let sans = leaf
		.subject_alternative_name()
		.map_err(|_| invalid("actor certificate has invalid subject alternative names"))?
		.ok_or_else(|| invalid("actor certificate has no subject alternative names"))?;
	let mut uris = sans
		.value
		.general_names
		.iter()
		.filter_map(|name| match name {
			GeneralName::URI(uri) => Some(uri),
			_ => None,
		});
	if uris.next().copied() != Some(expected_spiffe.as_str()) || uris.next().is_some() {
		return Err(invalid("actor certificate has an invalid SPIFFE identity"));
	}
	let client_auth = leaf.extensions().iter().any(|extension| {
		matches!(
			extension.parsed_extension(),
			ParsedExtension::ExtendedKeyUsage(usage) if usage.client_auth
		)
	});
	if !client_auth {
		return Err(invalid(
			"actor certificate leaf does not permit client authentication",
		));
	}
	const ACTOR_IDENTITY_OID: &str = "1.3.6.1.4.1.11129.2.12.2";
	let extensions = leaf
		.extensions()
		.iter()
		.filter(|extension| extension.oid.to_id_string() == ACTOR_IDENTITY_OID)
		.collect::<Vec<_>>();
	if extensions.len() != 1 {
		return Err(invalid(
			"actor certificate has an invalid identity extension",
		));
	}
	let extension = extensions[0];
	let actor: serde_json::Value = serde_json::from_slice(extension.value)
		.map_err(|_| invalid("actor certificate has an invalid identity extension"))?;
	if actor["Atespace"] != identity.atespace
		|| actor["ActorName"] != identity.actor_name
		|| actor["ActorUid"] != actor_uid
	{
		return Err(invalid(
			"actor certificate identity does not match the actor",
		));
	}
	Ok(())
}

fn credential_secret(secret: Vec<u8>) -> Result<Vec<u8>, ProxyError> {
	let secret = secret.strip_suffix(b"\n").unwrap_or(&secret);
	let secret = secret.strip_suffix(b"\r").unwrap_or(secret);
	if secret.is_empty() || secret.iter().any(|byte| byte.is_ascii_control()) {
		return Err(ProxyError::SubstrateEgressUnavailable(
			"credential provider returned an unusable secret".to_owned(),
		));
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
		name: "default".to_owned(),
		actor: Some(protos::ateapi::ObjectRef {
			atespace: identity.atespace.clone(),
			name: identity.actor_name.clone(),
		}),
	});
	let mut span = policy_client.start_grpc_span(
		&mut request,
		target.target.as_ref(),
		"/substrate.ateapi.v1.Control/GetActorEgressPolicy",
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
		return Err(ProxyError::SubstrateEgressDenied(
			"actor egress policy denied TLS destination".to_owned(),
		));
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
) -> Result<&'a protos::ateapi::EgressRule, ProxyError> {
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
		ProxyError::SubstrateEgressDenied("actor egress policy denied destination".to_owned())
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
	use crate::test_helpers::{ateapimock, credprovidermock, policy_client};

	#[tokio::test]
	async fn actor_jwt_cache_separates_bearer_and_rotated_certificates() {
		use std::sync::atomic::{AtomicUsize, Ordering};

		use protos::ateapi::{ActorJwtSource, MintActorJwtRequest, MintActorJwtResponse};

		struct Issuer(Arc<AtomicUsize>);

		#[async_trait::async_trait]
		impl ateapimock::Handler for Issuer {
			async fn mint_actor_jwt(
				&mut self,
				req: &MintActorJwtRequest,
			) -> Result<MintActorJwtResponse, tonic::Status> {
				let call = self.0.fetch_add(1, Ordering::Relaxed);
				Ok(MintActorJwtResponse {
					actor_jwt: format!("{call}:{:?}", req.client_certificate_sha256),
					..Default::default()
				})
			}
		}

		let calls = Arc::new(AtomicUsize::new(0));
		let api = ateapimock::AteApiMock::new({
			let calls = calls.clone();
			move || Issuer(calls.clone())
		})
		.spawn()
		.await;
		let policy: SubstrateEgress = serde_json::from_value(serde_json::json!({
			"host": api.address.to_string()
		}))
		.unwrap();
		let client = policy_client();
		let identity = ActorIdentity {
			atespace: "demo".into(),
			actor_name: "my-actor".into(),
			actor_uid: Some("uid-1".into()),
		};
		let source = ActorJwtSource {
			audiences: vec!["https://api.example.com".into()],
			expiration_seconds: 900,
		};
		let mint = |binding| policy.actor_jwt(&client, &identity, &source, binding);
		assert_eq!(mint(None).await.unwrap(), b"0:[]");
		for (call, sha256) in [(1, [1; 32]), (2, [2; 32])] {
			let binding = Some(sha256);
			let expected = format!("{call}:{sha256:?}").into_bytes();
			assert_eq!(mint(binding).await.unwrap(), expected);
			assert_eq!(mint(binding).await.unwrap(), expected);
		}
		assert_eq!(mint(None).await.unwrap(), b"0:[]");
		assert_eq!(calls.load(Ordering::Relaxed), 3);
	}

	#[tokio::test]
	async fn provider_client_certificate_caches_valid_bundles_and_retries_invalid_bundles() {
		use std::sync::atomic::{AtomicUsize, Ordering};

		use protos::ateapi::client_certificate_source::Source;
		use protos::ateapi::{ClientCertificateSource, CredentialClientCertificateSource};
		use protos::credprovider::{FetchSecretRequest, FetchSecretResponse};

		const URI: &str = "ate-secret://kubernetes.io/default/ns/cert/client.pem";
		const ACTOR: &str = "spiffe://substrate-actor.local/actor/demo/my-actor";

		struct BundleProvider {
			calls: Arc<AtomicUsize>,
			pem: Vec<u8>,
		}

		#[async_trait::async_trait]
		impl credprovidermock::Handler for BundleProvider {
			async fn fetch_secret(
				&mut self,
				req: &FetchSecretRequest,
			) -> Result<FetchSecretResponse, tonic::Status> {
				assert_eq!(req.uri, URI);
				assert_eq!(req.actor_spiffe_id, ACTOR);
				let call = self.calls.fetch_add(1, Ordering::Relaxed);
				Ok(FetchSecretResponse {
					opaque_bytes: if call == 0 {
						Vec::new()
					} else {
						self.pem.clone()
					},
				})
			}
		}

		let key = rcgen::KeyPair::generate().unwrap();
		let cert = rcgen::CertificateParams::default()
			.self_signed(&key)
			.unwrap();
		let pem = format!("{}{}", cert.pem(), key.serialize_pem()).into_bytes();
		let calls = Arc::new(AtomicUsize::new(0));
		let provider = credprovidermock::CredentialProviderMock::new({
			let calls = calls.clone();
			move || BundleProvider {
				calls: calls.clone(),
				pem: pem.clone(),
			}
		})
		.spawn()
		.await;

		let policy: SubstrateEgress = serde_json::from_value(serde_json::json!({
			"host": provider.address.to_string(),
			"credentialProviders": [{
				"uriAuthority": "kubernetes.io",
				"target": { "host": provider.address.to_string() }
			}]
		}))
		.unwrap();
		let client = policy_client();
		let identity = ActorIdentity {
			atespace: "demo".into(),
			actor_name: "my-actor".into(),
			actor_uid: Some("uid-1".into()),
		};
		let source = ClientCertificateSource {
			source: Some(Source::Credential(CredentialClientCertificateSource {
				credential_uri: URI.into(),
			})),
		};
		let fetch = || policy.client_certificate(&client, &identity, &source);
		assert!(matches!(
			fetch().await,
			Err(ProxyError::SubstrateEgressDenied(_))
		));
		assert!(matches!(
			fetch().await,
			Err(ProxyError::SubstrateEgressDenied(_))
		));
		assert_eq!(calls.load(Ordering::Relaxed), 1);

		let cache_key = CredentialCacheKey {
			actor_identity: ACTOR.into(),
			actor_uid: identity.actor_uid.clone(),
			uri: URI.into(),
		};
		assert!(
			policy
				.certificate_cache
				.get(&cache_key, Instant::now() + Duration::from_secs(5))
				.is_none()
		);
		let cert = fetch().await.unwrap();
		assert_eq!(fetch().await.unwrap().fingerprint, cert.fingerprint);
		assert_eq!(calls.load(Ordering::Relaxed), 2);
		assert!(
			policy
				.certificate_cache
				.get(&cache_key, Instant::now() + Duration::from_secs(5))
				.is_some()
		);

		let mut uncached = policy.clone();
		uncached.credential_providers[0].cache_ttl = Some(Duration::ZERO);
		uncached
			.client_certificate(&client, &identity, &source)
			.await
			.unwrap();
		uncached
			.client_certificate(&client, &identity, &source)
			.await
			.unwrap();
		assert_eq!(calls.load(Ordering::Relaxed), 4);
	}

	fn actor_certificate_params(actor_uid: &str, client_auth: bool) -> rcgen::CertificateParams {
		let mut params = rcgen::CertificateParams::default();
		params.subject_alt_names.push(rcgen::SanType::URI(
			"spiffe://substrate-actor.local/actor/demo/my-actor"
				.try_into()
				.unwrap(),
		));
		if client_auth {
			params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
		}
		params
			.custom_extensions
			.push(rcgen::CustomExtension::from_oid_content(
				&[1, 3, 6, 1, 4, 1, 11129, 2, 12, 2],
				serde_json::json!({
					"Atespace": "demo",
					"ActorName": "my-actor",
					"ActorUid": actor_uid,
				})
				.to_string()
				.into_bytes(),
			));
		params
	}

	fn minted_actor_certificate(
		actor_uid: &str,
		client_auth: bool,
	) -> (Vec<CertificateDer<'static>>, rcgen::KeyPair) {
		let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
		let certificate = actor_certificate_params(actor_uid, client_auth)
			.self_signed(&key)
			.unwrap();
		(vec![CertificateDer::from(certificate.der().to_vec())], key)
	}

	#[tokio::test]
	async fn actor_client_certificate_cache_refreshes_at_two_thirds_and_separates_uids() {
		use std::sync::atomic::{AtomicUsize, Ordering};

		use protos::ateapi::{MintActorCertificateRequest, MintActorCertificateResponse};

		struct Issuer(Arc<AtomicUsize>);

		#[async_trait::async_trait]
		impl ateapimock::Handler for Issuer {
			async fn mint_actor_certificate(
				&mut self,
				req: &MintActorCertificateRequest,
			) -> Result<MintActorCertificateResponse, tonic::Status> {
				self.0.fetch_add(1, Ordering::Relaxed);
				let mut csr = rcgen::CertificateSigningRequestParams::from_der(
					&req.certificate_signing_request.as_slice().into(),
				)
				.unwrap();
				csr.params = actor_certificate_params(&req.actor_uid, true);
				csr.params.not_before = std::time::SystemTime::now().into();
				csr.params.not_after = csr.params.not_before + Duration::from_secs(3600);
				let issuer = rcgen::Issuer::new(
					rcgen::CertificateParams::default(),
					rcgen::KeyPair::generate().unwrap(),
				);
				let cert = csr.signed_by(&issuer).unwrap();
				Ok(MintActorCertificateResponse {
					actor_certificates: vec![cert.der().to_vec()],
				})
			}
		}

		let calls = Arc::new(AtomicUsize::new(0));
		let api = ateapimock::AteApiMock::new({
			let calls = calls.clone();
			move || Issuer(calls.clone())
		})
		.spawn()
		.await;
		let policy: SubstrateEgress = serde_json::from_value(serde_json::json!({
			"host": api.address.to_string()
		}))
		.unwrap();
		let client = policy_client();
		let identity = ActorIdentity {
			atespace: "demo".into(),
			actor_name: "my-actor".into(),
			actor_uid: Some("uid-1".into()),
		};
		let fetch = |identity| policy.actor_client_certificate(&client, identity);
		let first = fetch(&identity).await.unwrap().leaf_sha256();
		assert_eq!(fetch(&identity).await.unwrap().leaf_sha256(), first);
		assert_eq!(calls.load(Ordering::Relaxed), 1);

		let recreated = ActorIdentity {
			actor_uid: Some("uid-2".into()),
			..identity.clone()
		};
		assert_ne!(fetch(&recreated).await.unwrap().leaf_sha256(), first);
		assert_eq!(calls.load(Ordering::Relaxed), 2);

		let cache_key = CredentialCacheKey {
			actor_identity: actor_spiffe_uri(&identity.atespace, &identity.actor_name),
			actor_uid: identity.actor_uid.clone(),
			uri: "actor-certificate".into(),
		};
		let now = Instant::now();
		assert!(
			policy
				.certificate_cache
				.get(&cache_key, now + Duration::from_secs(39 * 60))
				.is_some()
		);
		assert!(
			policy
				.certificate_cache
				.get(&cache_key, now + Duration::from_secs(41 * 60))
				.is_none()
		);
		assert_ne!(fetch(&identity).await.unwrap().leaf_sha256(), first);
		assert_eq!(calls.load(Ordering::Relaxed), 3);
	}

	#[test]
	fn minted_actor_client_certificate_requires_matching_actor_identity() {
		let identity = ActorIdentity {
			atespace: "demo".to_owned(),
			actor_name: "my-actor".to_owned(),
			actor_uid: Some("uid-1".to_owned()),
		};
		let (chain, key) = minted_actor_certificate("uid-1", true);
		validate_actor_client_certificate(&chain, &identity, "uid-1").unwrap();
		ClientCertificate::from_der(chain, PrivateKeyDer::Pkcs8(key.serialize_der().into())).unwrap();

		let (chain, _) = minted_actor_certificate("other-uid", true);
		assert!(validate_actor_client_certificate(&chain, &identity, "uid-1").is_err());
		let (chain, _) = minted_actor_certificate("uid-1", false);
		assert!(validate_actor_client_certificate(&chain, &identity, "uid-1").is_err());
	}

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
					upstream_tls: None,
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
	fn tls_rejects_invalid_sni_and_unmatched_destinations() {
		let policy = protos::ateapi::EgressPolicy {
			rules: vec![passthrough(&["*"], ports(&[443]))],
			..Default::default()
		};
		for sni in ["", "127.0.0.1", "bad..example", "example.com.."] {
			assert!(tls_mode(&policy, sni, 443).is_err(), "{sni}");
		}
		assert!(matches!(
			tls_mode(&policy, "api.example.com", 8443),
			Err(ProxyError::SubstrateEgressDenied(_))
		));
		for rules in [
			vec![],
			vec![passthrough(&["*"], None)],
			vec![rule(EgressRequestProtocol::Http, &["*"], all_ports())],
		] {
			let policy = protos::ateapi::EgressPolicy {
				rules,
				..Default::default()
			};
			assert!(matches!(
				tls_mode(&policy, "api.example.com", 443),
				Err(ProxyError::SubstrateEgressDenied(_))
			));
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
				actor_jwt: None,
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
				actor_jwt: None,
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
			actor_jwt: None,
		};
		assert!(credential_header(&injection, Vec::new()).is_err());
		assert!(credential_header(&injection, b"bad\nsecret".to_vec()).is_err());
	}

	#[test]
	fn credential_cache_reuses_fresh_entries_and_expires_stale_ones() {
		let cache = CredentialCache::new(16);
		let key = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/actor/default/example".to_owned(),
			actor_uid: Some("uid-1".to_owned()),
			uri: "ate-secret://kubernetes.io/default/token".to_owned(),
		};
		let now = Instant::now();
		cache.insert(
			key.clone(),
			b"token".to_vec(),
			now,
			DEFAULT_CREDENTIAL_CACHE_TTL,
		);
		assert_eq!(cache.get(&key, now), Some(b"token".to_vec()));
		assert_eq!(cache.get(&key, now + DEFAULT_CREDENTIAL_CACHE_TTL), None);

		cache.insert(key.clone(), b"uncached".to_vec(), now, Duration::ZERO);
		assert_eq!(cache.get(&key, now), None);
	}

	#[test]
	fn credential_cache_is_partitioned_by_actor_identity_uid_and_uri() {
		let cache = CredentialCache::new(16);
		let now = Instant::now();
		let key = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/actor/default/one".to_owned(),
			actor_uid: Some("uid-1".to_owned()),
			uri: "ate-secret://kubernetes.io/default/token".to_owned(),
		};
		cache.insert(
			key.clone(),
			b"one".to_vec(),
			now,
			DEFAULT_CREDENTIAL_CACHE_TTL,
		);

		let another_actor = CredentialCacheKey {
			actor_identity: "spiffe://substrate-actor.local/actor/default/two".to_owned(),
			..key.clone()
		};
		let another_uri = CredentialCacheKey {
			uri: "ate-secret://kubernetes.io/default/other".to_owned(),
			..key.clone()
		};
		let another_uid = CredentialCacheKey {
			actor_uid: Some("uid-2".to_owned()),
			..key
		};
		assert_eq!(cache.get(&another_actor, now), None);
		assert_eq!(cache.get(&another_uri, now), None);
		assert_eq!(cache.get(&another_uid, now), None);
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
