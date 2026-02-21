use std::cmp;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::{Arc, RwLock};

use anyhow::anyhow;
use hashbrown::Equivalent;
use heck::ToSnakeCase;
use itertools::Itertools;
use macro_rules_attribute::apply;
use openapiv3::OpenAPI;
use prometheus_client::encoding::EncodeLabelValue;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls_pemfile::Item;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::http::auth::BackendAuth;
use crate::http::authorization::RuleSet;
use crate::http::{
	HeaderOrPseudo, HeaderValue, ext_authz, ext_proc, filters, remoteratelimit, retry, timeout,
};
use crate::mcp::McpAuthorization;
use crate::telemetry::log::OrderedStringMap;
use crate::types::discovery::{NamespacedHostname, Service};
use crate::types::local::SimpleLocalBackend;
use crate::types::{agent, backend, frontend};
use crate::*;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bind {
	pub key: BindKey,
	pub address: SocketAddr,
	pub protocol: BindProtocol,
	pub tunnel_protocol: TunnelProtocol,
	pub listeners: ListenerSet,
}

pub type BindKey = Strng;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
	pub key: ListenerKey,
	// User facing name
	#[serde(flatten)]
	pub name: ListenerName,

	/// Can be a wildcard
	pub hostname: Strng,
	pub protocol: ListenerProtocol,
	pub routes: RouteSet,
	pub tcp_routes: TCPRouteSet,
}

impl Listener {
	pub fn matches(&self, hostname: &str) -> bool {
		self.hostname == hostname
			|| self.hostname.is_empty()
			|| (self.hostname.starts_with("*") && hostname.ends_with(&self.hostname[1..]))
	}
}

type Alpns = Vec<Vec<u8>>;

#[derive(Debug, Clone)]
struct ServerTlsInputs {
	cert_pem: Vec<u8>,
	key_pem: Vec<u8>,
	// If present, require and verify client certificates using these roots.
	root_pem: Option<Vec<u8>>,
	// Default ALPNs configured at creation time.
	default_alpns: Alpns,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ServerTlsProfileKey {
	alpns: Alpns,
	min_version: Option<TLSVersion>,
	max_version: Option<TLSVersion>,
	// Order-sensitive: we intentionally preserve user-provided cipher suite ordering.
	cipher_suites: Vec<crate::transport::tls::CipherSuite>,
}

impl frontend::TLS {
	/// Fast path: no overrides
	fn is_fast_path(&self) -> bool {
		// empty list is the same as no overrides
		let no_cipher_suite_override = self
			.cipher_suites
			.as_deref()
			.is_none_or(|suites| suites.is_empty());

		self.alpn.is_none()
			&& self.min_version.is_none()
			&& self.max_version.is_none()
			&& no_cipher_suite_override
	}

	fn server_tls_profile_key(&self, default_alpns: &Alpns) -> ServerTlsProfileKey {
		let alpns = self.alpn.clone().unwrap_or_else(|| default_alpns.clone());
		let min_version = self.min_version.map(Into::into);
		let max_version = self.max_version.map(Into::into);
		let cipher_suites = self.cipher_suites.clone().unwrap_or_default();
		ServerTlsProfileKey {
			alpns,
			min_version,
			max_version,
			cipher_suites,
		}
	}
}

#[derive(Debug, Clone)]
pub struct ServerTLSConfig {
	/// Cached base config (built from `inputs` using defaults). Kept for fast path when no overrides
	/// are requested.
	base_config: Option<Arc<ServerConfig>>,
	/// Original inputs required to rebuild a fresh `ServerConfig` for a given profile.
	inputs: Option<Arc<ServerTlsInputs>>,
	per_profile_config: Arc<RwLock<HashMap<ServerTlsProfileKey, Arc<ServerConfig>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
#[allow(non_camel_case_types)]
pub enum TLSVersion {
	TLS_V1_0,
	TLS_V1_1,
	TLS_V1_2,
	TLS_V1_3,
}

impl ServerTLSConfig {
	pub fn new(config: Arc<ServerConfig>) -> Self {
		Self {
			base_config: Some(config),
			inputs: None,
			per_profile_config: Arc::new(Default::default()),
		}
	}

	pub fn from_pem(
		cert_pem: Vec<u8>,
		key_pem: Vec<u8>,
		root_pem: Option<Vec<u8>>,
		default_alpns: Alpns,
	) -> anyhow::Result<Self> {
		Self::from_pem_with_profile(cert_pem, key_pem, root_pem, default_alpns, None, None, None)
	}

	pub fn from_pem_with_profile(
		cert_pem: Vec<u8>,
		key_pem: Vec<u8>,
		root_pem: Option<Vec<u8>>,
		default_alpns: Alpns,
		min_version: Option<TLSVersion>,
		max_version: Option<TLSVersion>,
		cipher_suites: Option<Vec<crate::transport::tls::CipherSuite>>,
	) -> anyhow::Result<Self> {
		let inputs = Arc::new(ServerTlsInputs {
			cert_pem,
			key_pem,
			root_pem,
			default_alpns,
		});
		let suites = cipher_suites.as_deref().filter(|s| !s.is_empty());
		let base = Arc::new(Self::build_server_config(
			&inputs,
			None,
			min_version,
			max_version,
			suites.unwrap_or(&[]),
		)?);
		Ok(Self {
			base_config: Some(base),
			inputs: Some(inputs),
			per_profile_config: Arc::new(Default::default()),
		})
	}

	/// new_invalid returns a ServerTLSConfig that always rejects connections
	pub fn new_invalid() -> Self {
		Self {
			base_config: None,
			inputs: None,
			per_profile_config: Arc::new(Default::default()),
		}
	}
	/// config_for returns the appropriate config for the requested ALPN
	/// If none is return, it means the certificates were invalid.
	pub fn config_for(&self, tls: Option<&frontend::TLS>) -> anyhow::Result<Arc<ServerConfig>> {
		let inputs = match self.inputs.as_ref() {
			Some(i) => Arc::clone(i),
			None => {
				return self
					.base_config
					.clone()
					.ok_or_else(|| anyhow!("TLS config invalid"));
			},
		};

		// Fast path: no overrides
		if tls.is_none_or(|t| t.is_fast_path())
			&& let Some(c) = self.base_config.clone()
		{
			return Ok(c);
		}

		let key = match tls {
			Some(tls) => tls.server_tls_profile_key(&inputs.default_alpns),
			None => ServerTlsProfileKey {
				alpns: inputs.default_alpns.clone(),
				min_version: None,
				max_version: None,
				cipher_suites: vec![],
			},
		};

		{
			let reader = self.per_profile_config.read().unwrap();
			if let Some(cached_config) = reader.get(&key) {
				return Ok(Arc::clone(cached_config));
			}
		}
		let mut writer = self.per_profile_config.write().unwrap();
		if let Some(cached_config) = writer.get(&key) {
			return Ok(Arc::clone(cached_config));
		}

		let built = Arc::new(Self::build_server_config(
			&inputs,
			Some(&key.alpns),
			key.min_version,
			key.max_version,
			&key.cipher_suites,
		)?);
		writer.insert(key, Arc::clone(&built));
		Ok(built)
	}

	fn build_server_config(
		inputs: &ServerTlsInputs,
		alpns: Option<&[Vec<u8>]>,
		min_version: Option<TLSVersion>,
		max_version: Option<TLSVersion>,
		cipher_suites: &[crate::transport::tls::CipherSuite],
	) -> anyhow::Result<ServerConfig> {
		let provider = if cipher_suites.is_empty() {
			crate::transport::tls::provider()
		} else {
			crate::transport::tls::provider_with_cipher_suites(cipher_suites)?
		};

		let versions = tls_versions_for_range(min_version, max_version)?;
		let scb = ServerConfig::builder_with_provider(provider.clone())
			.with_protocol_versions(&versions)
			.expect("server config must be valid");

		let scb = if let Some(root) = &inputs.root_pem {
			let mut roots_store = rustls::RootCertStore::empty();
			let mut reader = std::io::BufReader::new(Cursor::new(root.as_slice()));
			let certs = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
			roots_store.add_parsable_certificates(certs);
			let verify = rustls::server::WebPkiClientVerifier::builder_with_provider(
				Arc::new(roots_store),
				provider,
			)
			.build()?;
			scb.with_client_cert_verifier(verify)
		} else {
			scb.with_no_client_auth()
		};

		let cert_chain = parse_cert(&inputs.cert_pem)?;
		let private_key = parse_key(&inputs.key_pem)?;
		let mut sc = scb.with_single_cert(cert_chain, private_key)?;
		sc.alpn_protocols = alpns
			.map(|a| a.to_vec())
			.unwrap_or_else(|| inputs.default_alpns.clone());
		Ok(sc)
	}
}

fn tls_versions_for_range(
	min_version: Option<TLSVersion>,
	max_version: Option<TLSVersion>,
) -> anyhow::Result<Vec<&'static rustls::SupportedProtocolVersion>> {
	// rustls currently supports TLS1.2 and TLS1.3 in this repo (see `transport::tls::ALL_TLS_VERSIONS`).
	// If older versions are requested, reject early.
	fn ord(v: TLSVersion) -> anyhow::Result<u8> {
		match v {
			TLSVersion::TLS_V1_2 => Ok(12),
			TLSVersion::TLS_V1_3 => Ok(13),
			_ => Err(anyhow!("unsupported TLS version: {v:?}")),
		}
	}

	let min = min_version.map(ord).transpose()?;
	let max = max_version.map(ord).transpose()?;
	if let (Some(min), Some(max)) = (min, max)
		&& min > max
	{
		return Err(anyhow!("invalid TLS version range"));
	}

	let min = min.unwrap_or(12);
	let max = max.unwrap_or(13);

	let mut out = Vec::new();
	if min <= 12 && max >= 12 {
		out.push(&rustls::version::TLS12);
	}
	if min <= 13 && max >= 13 {
		out.push(&rustls::version::TLS13);
	}
	if out.is_empty() {
		return Err(anyhow!("invalid TLS version range"));
	}
	Ok(out)
}

impl serde::Serialize for ServerTLSConfig {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		// TODO: store raw pem
		serializer.serialize_none()
	}
}

pub fn parse_cert(mut cert: &[u8]) -> Result<Vec<CertificateDer<'static>>, anyhow::Error> {
	let mut reader = std::io::BufReader::new(Cursor::new(&mut cert));
	let parsed: Result<Vec<_>, _> = rustls_pemfile::read_all(&mut reader).collect();
	parsed?
		.into_iter()
		.map(|p| {
			let Item::X509Certificate(der) = p else {
				return Err(anyhow!("no certificate"));
			};
			Ok(der)
		})
		.collect::<Result<Vec<_>, _>>()
}

pub fn parse_key(mut key: &[u8]) -> Result<PrivateKeyDer<'static>, anyhow::Error> {
	let mut reader = std::io::BufReader::new(Cursor::new(&mut key));
	let parsed = rustls_pemfile::read_one(&mut reader)?;
	let parsed = parsed.ok_or_else(|| anyhow!("no key"))?;
	match parsed {
		Item::Pkcs8Key(c) => Ok(PrivateKeyDer::Pkcs8(c)),
		Item::Pkcs1Key(c) => Ok(PrivateKeyDer::Pkcs1(c)),
		Item::Sec1Key(c) => Ok(PrivateKeyDer::Sec1(c)),
		_ => Err(anyhow!("unsupported key")),
	}
}
#[derive(Debug, Clone, serde::Serialize)]
pub enum ListenerProtocol {
	/// HTTP
	HTTP,
	/// HTTPS, terminating TLS then treating as HTTP
	HTTPS(ServerTLSConfig),
	/// TLS (passthrough or termination)
	TLS(Option<ServerTLSConfig>),
	/// Opaque TCP
	TCP,
	HBONE,
}

impl ListenerProtocol {
	pub fn tls(
		&self,
		tls: Option<&frontend::TLS>,
	) -> Option<anyhow::Result<Arc<rustls::ServerConfig>>> {
		match self {
			ListenerProtocol::HTTPS(t) => Some(t.config_for(tls)),
			ListenerProtocol::TLS(t) => t.as_ref().map(|t| t.config_for(tls)),
			_ => None,
		}
	}
}

// Protocol of the entire bind.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, EncodeLabelValue, Serialize)]
#[allow(non_camel_case_types)]
pub enum BindProtocol {
	http,
	// Note: TLS can be TLS (passthrough or termination) or HTTPS
	tls,
	tcp,
}

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq, Hash, EncodeLabelValue)]
pub enum TunnelProtocol {
	#[default]
	Direct,
	HboneWaypoint,
	HboneGateway,
	Proxy,
}

// Protocol of the request
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, EncodeLabelValue)]
#[allow(non_camel_case_types)]
pub enum TransportProtocol {
	http,
	https,
	hbone,
	tcp,
	tls,
}

pub type ListenerKey = Strng;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
	// Internal name
	pub key: RouteKey,
	#[serde(flatten)]
	// User facing name of the route
	pub name: RouteName,
	/// Can be a wildcard
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub hostnames: Vec<Strng>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub matches: Vec<RouteMatch>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub backends: Vec<RouteBackendReference>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<TrafficPolicy>,
}

pub type RouteKey = Strng;
pub type RouteRuleName = Strng;

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
#[cfg_attr(any(test, feature = "internal_benches"), derive(Default))]
pub struct RouteName {
	pub name: Strng,
	pub namespace: Strng,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rule_name: Option<Strng>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub kind: Option<Strng>,
}

impl RouteName {
	pub fn as_route_name(&self) -> Strng {
		strng::format!("{}/{}", self.namespace, self.name)
	}
	pub fn as_route_target_ref(&self) -> PolicyTargetRef {
		PolicyTargetRef::Route {
			name: self.name.as_ref(),
			namespace: self.namespace.as_ref(),
			rule_name: None,
			kind: self.kind.as_deref(),
		}
	}
	pub fn as_route_rule_target_ref(&self) -> PolicyTargetRef {
		PolicyTargetRef::Route {
			name: self.name.as_ref(),
			namespace: self.namespace.as_ref(),
			rule_name: self.rule_name.as_deref(),
			kind: self.kind.as_deref(),
		}
	}
}

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
#[cfg_attr(any(test, feature = "internal_benches"), derive(Default))]
pub struct ListenerName {
	pub gateway_name: Strng,
	pub gateway_namespace: Strng,
	pub listener_name: Strng,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub listener_set: Option<ResourceName>,
}

impl ListenerName {
	pub fn as_gateway_name(&self) -> Strng {
		strng::format!("{}/{}", self.gateway_namespace, self.gateway_name)
	}
	pub fn as_gateway_target_ref(&self) -> PolicyTargetRef {
		PolicyTargetRef::Gateway {
			gateway_name: self.gateway_name.as_ref(),
			gateway_namespace: self.gateway_namespace.as_ref(),
			listener_name: None,
		}
	}
	pub fn as_listener_target_ref(&self) -> PolicyTargetRef {
		PolicyTargetRef::Gateway {
			gateway_name: self.gateway_name.as_ref(),
			gateway_namespace: self.gateway_namespace.as_ref(),
			listener_name: Some(self.listener_name.as_ref()),
		}
	}
}

impl From<ListenerName> for ListenerTarget {
	fn from(l: ListenerName) -> Self {
		Self {
			gateway_name: l.gateway_name.clone(),
			gateway_namespace: l.gateway_namespace.clone(),
			listener_name: Some(l.listener_name.clone()),
		}
	}
}

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
pub struct ListenerTarget {
	pub gateway_name: Strng,
	pub gateway_namespace: Strng,
	pub listener_name: Option<Strng>,
}

impl ListenerTarget {
	pub fn strip_listener_fields(&self) -> ListenerTarget {
		Self {
			gateway_name: self.gateway_name.clone(),
			gateway_namespace: self.gateway_namespace.clone(),
			listener_name: None,
		}
	}
}

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
pub struct ResourceName {
	pub name: Strng,
	pub namespace: Strng,
}

impl ResourceName {
	pub fn new(name: Strng, namespace: Strng) -> Self {
		Self { name, namespace }
	}
}

impl fmt::Display for ResourceName {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}/{}", self.namespace, self.name)
	}
}

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
pub struct TypedResourceName {
	pub kind: Strng,
	pub name: Strng,
	pub namespace: Strng,
}

impl fmt::Display for TypedResourceName {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}/{}/{}", self.kind, self.namespace, self.name)
	}
}

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
pub enum BackendTarget {
	Backend {
		name: Strng,
		namespace: Strng,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		section: Option<Strng>,
	},
	Service {
		hostname: Strng,
		namespace: Strng,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		port: Option<u16>,
	},
	Invalid,
}

#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub enum BackendTargetRef<'a> {
	Backend {
		name: &'a str,
		namespace: &'a str,
		section: Option<&'a str>,
	},
	Service {
		hostname: &'a str,
		namespace: &'a str,
		port: Option<u16>,
	},
	Invalid,
}

impl<'a> From<&'a BackendTarget> for BackendTargetRef<'a> {
	fn from(value: &'a BackendTarget) -> Self {
		match value {
			BackendTarget::Backend {
				name,
				namespace,
				section,
			} => BackendTargetRef::Backend {
				name,
				namespace,
				section: section.as_deref(),
			},
			BackendTarget::Service {
				hostname,
				namespace,
				port,
			} => BackendTargetRef::Service {
				hostname,
				namespace,
				port: *port,
			},
			BackendTarget::Invalid => BackendTargetRef::Invalid,
		}
	}
}

impl BackendTargetRef<'_> {
	pub fn strip_section(&self) -> BackendTargetRef {
		match self {
			BackendTargetRef::Backend {
				name, namespace, ..
			} => BackendTargetRef::Backend {
				name,
				namespace,
				section: None,
			},
			BackendTargetRef::Service {
				namespace,
				hostname,
				..
			} => BackendTargetRef::Service {
				namespace,
				hostname,
				port: None,
			},
			BackendTargetRef::Invalid => BackendTargetRef::Invalid,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TCPRoute {
	// Internal name
	pub key: RouteKey,
	// User facing name of the route
	#[serde(flatten)]
	pub name: RouteName,
	// Can be a wildcard. Not applicable for TCP, only for TLS
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub hostnames: Vec<Strng>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub backends: Vec<TCPRouteBackendReference>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TCPRouteBackendReference {
	#[serde(default = "default_weight")]
	pub weight: usize,
	pub backend: SimpleBackendReference,
	// Inline policies ("filters") of the route backend
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendPolicy>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TCPRouteBackend {
	#[serde(default = "default_weight")]
	pub weight: usize,
	pub backend: SimpleBackendWithPolicies,
	// Inline policies ("filters") of the route backend
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendPolicy>,
}

#[apply(schema!)]
pub struct RouteMatch {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub headers: Vec<HeaderMatch>,
	pub path: PathMatch,
	#[serde(default, flatten, skip_serializing_if = "Option::is_none")]
	pub method: Option<MethodMatch>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub query: Vec<QueryMatch>,
}

#[apply(schema!)]
pub struct MethodMatch {
	pub method: Strng,
}

#[apply(schema!)]
pub struct HeaderMatch {
	#[serde(serialize_with = "ser_display", deserialize_with = "de_parse")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	pub name: HeaderOrPseudo,
	pub value: HeaderValueMatch,
}

#[apply(schema!)]
pub struct QueryMatch {
	#[serde(serialize_with = "ser_display")]
	pub name: Strng,
	pub value: QueryValueMatch,
}

#[apply(schema!)]
pub enum QueryValueMatch {
	Exact(Strng),
	Regex(
		#[serde(with = "serde_regex")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		regex::Regex,
	),
}

#[apply(schema!)]
pub enum HeaderValueMatch {
	Exact(
		#[serde(serialize_with = "ser_bytes", deserialize_with = "de_parse")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		HeaderValue,
	),
	Regex(
		#[serde(with = "serde_regex")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		regex::Regex,
	),
}

#[apply(schema!)]
pub enum PathMatch {
	Exact(Strng),
	PathPrefix(Strng),
	Regex(
		#[serde(with = "serde_regex")]
		#[cfg_attr(feature = "schema", schemars(with = "String"))]
		regex::Regex,
		usize,
	),
}

#[apply(schema!)]
#[derive(Eq, PartialEq)]
pub enum HostRedirect {
	Full(Strng),
	Host(Strng),
	Port(NonZeroU16),
	Auto,
	None,
}

#[apply(schema!)]
#[derive(Eq, PartialEq, Copy)]
pub enum HostRedirectOverride {
	Auto,
	None,
}

#[apply(schema!)]
#[derive(Eq, PartialEq)]
pub enum PathRedirect {
	Full(Strng),
	Prefix(Strng),
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteBackendReference {
	#[serde(default = "default_weight")]
	pub weight: usize,
	#[serde(flatten)]
	pub backend: BackendReference,
	// Inline policies ("filters") of the route backend
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendPolicy>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteBackend {
	#[serde(default = "default_weight")]
	pub weight: usize,
	pub backend: BackendWithPolicies,
	// Inline policies ("filters") of the route backend
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendPolicy>,
}

#[allow(unused)]
fn default_weight() -> usize {
	1
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendWithPolicies {
	pub backend: Backend,

	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendPolicy>,
}

impl From<SimpleBackendWithPolicies> for BackendWithPolicies {
	fn from(backend: SimpleBackendWithPolicies) -> Self {
		Self {
			backend: Backend::from(backend.backend),
			inline_policies: backend.inline_policies,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
	Service(Arc<Service>, u16),
	#[serde(rename = "host", serialize_with = "serialize_backend_tuple")]
	Opaque(ResourceName, Target), // Hostname or IP
	#[serde(rename = "mcp", serialize_with = "serialize_backend_tuple")]
	MCP(ResourceName, McpBackend),
	#[serde(rename = "ai", serialize_with = "serialize_backend_tuple")]
	AI(ResourceName, crate::llm::AIBackend),
	#[serde(serialize_with = "serialize_backend_tuple")]
	Dynamic(ResourceName, ()),
	Invalid,
}

impl From<Backend> for BackendWithPolicies {
	fn from(val: Backend) -> Self {
		BackendWithPolicies {
			backend: val,
			inline_policies: vec![],
		}
	}
}

pub fn serialize_backend_tuple<S: Serializer, T: serde::Serialize>(
	name: &ResourceName,
	t: T,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	#[derive(Debug, Clone, serde::Serialize)]
	#[serde(rename_all = "camelCase")]
	struct BackendTuple<'a, T: serde::Serialize> {
		#[serde(flatten)]
		name: &'a ResourceName,
		target: &'a T,
	}
	BackendTuple { name, target: &t }.serialize(serializer)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendReference {
	Service { name: NamespacedHostname, port: u16 },
	Backend(BackendKey),
	Invalid,
}

impl From<SimpleBackend> for Backend {
	fn from(value: SimpleBackend) -> Self {
		match value {
			SimpleBackend::Service(svc, port) => Backend::Service(svc, port),
			SimpleBackend::Opaque(name, target) => Backend::Opaque(name, target),
			SimpleBackend::Invalid => Backend::Invalid,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SimpleBackend {
	Service(Arc<Service>, u16),
	#[serde(rename = "host")]
	Opaque(ResourceName, Target), // Hostname or IP
	Invalid,
}

impl fmt::Display for SimpleBackend {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			SimpleBackend::Service(service, port) => write!(f, "{}:{}", service.hostname, port),
			SimpleBackend::Opaque(name, _) => write!(f, "{}", name),
			SimpleBackend::Invalid => write!(f, "invalid"),
		}
	}
}

impl TryFrom<Backend> for SimpleBackend {
	type Error = anyhow::Error;

	fn try_from(value: Backend) -> Result<Self, Self::Error> {
		match value {
			Backend::Service(svc, port) => Ok(SimpleBackend::Service(svc, port)),
			Backend::Opaque(name, tgt) => Ok(SimpleBackend::Opaque(name, tgt)),
			Backend::Invalid => Ok(SimpleBackend::Invalid),
			_ => anyhow::bail!("unsupported backend type"),
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleBackendWithPolicies {
	pub backend: SimpleBackend,

	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<BackendPolicy>,
}

impl From<SimpleBackend> for SimpleBackendWithPolicies {
	fn from(value: SimpleBackend) -> Self {
		Self {
			backend: value,
			inline_policies: vec![],
		}
	}
}

#[derive(Eq, PartialEq)]
#[apply(schema_ser!)]
#[cfg_attr(feature = "schema", schemars(with = "SimpleLocalBackend"))]
pub enum SimpleBackendReference {
	Service { name: NamespacedHostname, port: u16 },
	Backend(BackendKey),
	InlineBackend(Target),
	Invalid,
}

impl<'de> serde::Deserialize<'de> for SimpleBackendReference {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let slb = SimpleLocalBackend::deserialize(deserializer)?;
		match slb {
			SimpleLocalBackend::Service { name, port } => {
				Ok(SimpleBackendReference::Service { name, port })
			},
			SimpleLocalBackend::Opaque(t) => Ok(SimpleBackendReference::InlineBackend(t)),
			SimpleLocalBackend::Backend(n) => Ok(SimpleBackendReference::Backend(n)),
			SimpleLocalBackend::Invalid => Ok(SimpleBackendReference::Invalid),
		}
	}
}

impl SimpleBackend {
	pub fn hostport(&self) -> String {
		match self {
			SimpleBackend::Service(svc, port) => {
				format!("{}:{port}", svc.hostname)
			},
			SimpleBackend::Opaque(_, tgt) => tgt.hostport(),
			SimpleBackend::Invalid => "invalid".to_string(),
		}
	}

	pub fn target(&self) -> BackendTargetRef {
		match self {
			SimpleBackend::Service(svc, port) => BackendTargetRef::Service {
				hostname: svc.hostname.as_ref(),
				namespace: svc.namespace.as_ref(),
				port: Some(*port),
			},
			SimpleBackend::Opaque(name, _) => BackendTargetRef::Backend {
				name: name.name.as_ref(),
				namespace: name.namespace.as_ref(),
				section: None,
			},
			SimpleBackend::Invalid => BackendTargetRef::Invalid,
		}
	}

	pub fn backend_type(&self) -> cel::BackendType {
		match self {
			SimpleBackend::Service(_, _) => cel::BackendType::Service,
			SimpleBackend::Opaque(_, _) => cel::BackendType::Static,
			SimpleBackend::Invalid => cel::BackendType::Unknown,
		}
	}

	pub fn backend_info(&self) -> BackendInfo {
		BackendInfo {
			backend_type: self.backend_type(),
			backend_name: strng::format!("{}", self),
		}
	}
}

impl Backend {
	pub fn target(&self) -> BackendTarget {
		match self {
			Backend::Service(svc, port) => BackendTarget::Service {
				hostname: svc.hostname.clone(),
				namespace: svc.namespace.clone(),
				port: Some(*port),
			},
			Backend::Opaque(name, _)
			| Backend::MCP(name, _)
			| Backend::AI(name, _)
			| Backend::Dynamic(name, _) => BackendTarget::Backend {
				name: name.name.clone(),
				namespace: name.namespace.clone(),
				section: None,
			},
			Backend::Invalid => BackendTarget::Invalid,
		}
	}

	pub fn target_ref(&self) -> BackendTargetRef {
		match self {
			Backend::Service(svc, port) => BackendTargetRef::Service {
				hostname: svc.hostname.as_ref(),
				namespace: svc.namespace.as_ref(),
				port: Some(*port),
			},
			Backend::Opaque(name, _)
			| Backend::MCP(name, _)
			| Backend::AI(name, _)
			| Backend::Dynamic(name, _) => BackendTargetRef::Backend {
				name: name.name.as_ref(),
				namespace: name.namespace.as_ref(),
				section: None,
			},
			Backend::Invalid => BackendTargetRef::Invalid,
		}
	}

	pub fn name(&self) -> Strng {
		match self {
			Backend::Service(svc, port) => strng::format!("{}:{}", svc.hostname.clone(), port),
			Backend::Opaque(name, _)
			| Backend::MCP(name, _)
			| Backend::AI(name, _)
			| Backend::Dynamic(name, _) => strng::format!("{}", name),
			Backend::Invalid => strng::literal!("invalid"),
		}
	}

	pub fn backend_type(&self) -> cel::BackendType {
		match self {
			Backend::Service(_, _) => cel::BackendType::Service,
			Backend::Opaque(_, _) => cel::BackendType::Static,
			Backend::MCP(_, _) => cel::BackendType::MCP,
			Backend::AI(_, _) => cel::BackendType::AI,
			Backend::Dynamic { .. } => cel::BackendType::Dynamic,
			Backend::Invalid => cel::BackendType::Unknown,
		}
	}

	pub fn backend_protocol(&self) -> Option<cel::BackendProtocol> {
		match self {
			Backend::MCP(_, _) => Some(cel::BackendProtocol::mcp),
			Backend::AI(_, _) => Some(cel::BackendProtocol::llm),
			_ => None,
		}
	}

	pub fn backend_info(&self) -> BackendInfo {
		BackendInfo {
			backend_type: self.backend_type(),
			backend_name: self.name(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct BackendInfo {
	pub backend_type: cel::BackendType,
	pub backend_name: Strng,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct McpBackend {
	pub targets: Vec<Arc<McpTarget>>,
	pub stateful: bool,
	pub always_use_prefix: bool,
}

impl McpBackend {
	pub fn find(&self, name: &str) -> Option<Arc<McpTarget>> {
		self
			.targets
			.iter()
			.find(|target| target.name.as_str() == name)
			.cloned()
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct McpTarget {
	pub name: McpTargetName,
	#[serde(flatten)]
	pub spec: McpTargetSpec,
}

pub type McpTargetName = Strng;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum McpTargetSpec {
	#[serde(rename = "sse")]
	Sse(SseTargetSpec),
	#[serde(rename = "mcp")]
	Mcp(StreamableHTTPTargetSpec),
	#[serde(rename = "stdio")]
	Stdio {
		cmd: String,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		args: Vec<String>,
		#[serde(default, skip_serializing_if = "HashMap::is_empty")]
		env: HashMap<String, String>,
	},
	#[serde(rename = "openapi")]
	OpenAPI(OpenAPITarget),
}

impl McpTargetSpec {
	pub fn backend(&self) -> Option<&SimpleBackendReference> {
		match self {
			McpTargetSpec::Sse(s) => Some(&s.backend),
			McpTargetSpec::Mcp(s) => Some(&s.backend),
			McpTargetSpec::OpenAPI(s) => Some(&s.backend),
			McpTargetSpec::Stdio { .. } => None,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct SseTargetSpec {
	pub backend: SimpleBackendReference,
	pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct StreamableHTTPTargetSpec {
	pub backend: SimpleBackendReference,
	pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct OpenAPITarget {
	pub backend: SimpleBackendReference,
	#[serde(deserialize_with = "de_openapi")]
	#[cfg_attr(feature = "schema", schemars(with = "serde_json::value::RawValue"))]
	pub schema: Arc<OpenAPI>,
}

pub fn de_openapi<'a, D>(deserializer: D) -> Result<Arc<OpenAPI>, D::Error>
where
	D: serde::Deserializer<'a>,
{
	#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
	#[serde(rename_all = "camelCase", deny_unknown_fields)]
	enum Serde {
		File(PathBuf),
		Inline(String),
		// Remote()
	}
	let s = Serde::deserialize(deserializer)?;

	let s = match s {
		Serde::File(f) => {
			let f = std::fs::read(f).map_err(serde::de::Error::custom)?;
			String::from_utf8(f).map_err(serde::de::Error::custom)?
		},
		Serde::Inline(s) => s,
	};
	// OpenAPI can be huge, so grow our stack
	let schema: OpenAPI = stacker::grow(2 * 1024 * 1024, || {
		yamlviajson::from_str(s.as_str()).map_err(serde::de::Error::custom)
	})?;

	Ok(Arc::new(schema))
}

#[derive(Debug, Clone, Default)]
pub struct ListenerSet {
	pub inner: HashMap<ListenerKey, Arc<Listener>>,
}

impl serde::Serialize for ListenerSet {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.inner.serialize(serializer)
	}
}

impl ListenerSet {
	pub fn from_list<const N: usize>(l: [Listener; N]) -> ListenerSet {
		let mut listeners = HashMap::with_capacity(l.len());
		for ls in l.into_iter() {
			listeners.insert(ls.key.clone(), Arc::new(ls));
		}
		ListenerSet { inner: listeners }
	}

	pub fn best_match(&self, host: &str) -> Option<Arc<Listener>> {
		if let Some(best) = self.inner.values().find(|l| l.hostname == host) {
			trace!("found best match for {host} (exact)");
			return Some(best.clone());
		}
		if let Some(best) = self
			.inner
			.values()
			.sorted_by_key(|l| -(l.hostname.len() as i64))
			.find(|l| l.hostname.starts_with("*") && host.ends_with(&l.hostname.as_str()[1..]))
		{
			trace!("found best match for {host} (wildcard {})", best.hostname);
			return Some(best.clone());
		}
		trace!("trying to find best match for {host} (empty hostname)");
		self.inner.values().find(|l| l.hostname.is_empty()).cloned()
	}

	pub fn insert(&mut self, v: Listener) {
		self.inner.insert(v.key.clone(), Arc::new(v));
	}

	pub fn contains(&self, key: &ListenerKey) -> bool {
		self.inner.contains_key(key)
	}

	pub fn get(&self, key: &ListenerKey) -> Option<&Listener> {
		self.inner.get(key).map(Arc::as_ref)
	}

	pub fn get_exactly_one(&self) -> anyhow::Result<Arc<Listener>> {
		if self.inner.len() != 1 {
			anyhow::bail!("expecting only one listener for TCP");
		}
		self
			.inner
			.iter()
			.next()
			.ok_or_else(|| anyhow::anyhow!("expecting one listener"))
			.map(|(_k, v)| v.clone())
	}

	pub fn remove(&mut self, key: &ListenerKey) -> Option<Arc<Listener>> {
		self.inner.remove(key)
	}

	pub fn iter(&self) -> impl Iterator<Item = &Listener> {
		self.inner.values().map(Arc::as_ref)
	}
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize)]
pub enum HostnameMatch {
	Exact(Strng),
	// *.example.com -> Wildcard(example.com)
	Wildcard(Strng),
	None,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize)]
pub enum HostnameMatchRef<'a> {
	Exact(&'a str),
	// *.example.com -> Wildcard(example.com)
	Wildcard(&'a str),
	None,
}
impl Equivalent<HostnameMatch> for HostnameMatchRef<'_> {
	fn equivalent(&self, key: &HostnameMatch) -> bool {
		self == &HostnameMatchRef::from(key)
	}
}

impl<'a> From<&'a HostnameMatch> for HostnameMatchRef<'a> {
	fn from(value: &'a HostnameMatch) -> Self {
		match value {
			HostnameMatch::Exact(e) => HostnameMatchRef::Exact(e.as_str()),
			HostnameMatch::Wildcard(w) => HostnameMatchRef::Wildcard(w.as_str()),
			HostnameMatch::None => HostnameMatchRef::None,
		}
	}
}

impl From<Strng> for HostnameMatch {
	fn from(s: Strng) -> Self {
		if let Some(s) = s.strip_prefix("*.") {
			HostnameMatch::Wildcard(strng::new(s))
		} else {
			HostnameMatch::Exact(s.clone())
		}
	}
}

impl HostnameMatch {
	pub fn all_matches_or_none<'a>(
		hostname: Option<&'a str>,
	) -> Box<dyn Iterator<Item = HostnameMatchRef<'a>> + '_> {
		match hostname {
			None => Box::new(std::iter::once(HostnameMatchRef::None)),
			Some(h) => Box::new(Self::all_matches(h)),
		}
	}
	pub fn all_matches<'a>(hostname: &'a str) -> impl Iterator<Item = HostnameMatchRef<'a>> + '_ {
		Self::all_actual_matches(hostname).chain(std::iter::once(HostnameMatchRef::None))
	}
	fn all_actual_matches<'a>(hostname: &'a str) -> impl Iterator<Item = HostnameMatchRef<'a>> + '_ {
		let has_wildcard_prefix = hostname.starts_with("*.");

		let exact_match = if has_wildcard_prefix {
			None
		} else {
			Some(HostnameMatchRef::Exact(hostname))
		};

		let wildcards = hostname.char_indices().filter_map(move |(i, c)| {
			if c == '.' {
				Some(HostnameMatchRef::Wildcard(&hostname[i + 1..]))
			} else {
				None
			}
		});

		exact_match.into_iter().chain(wildcards)
	}
}

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize)]
pub struct SingleRouteMatch {
	key: RouteKey,
	index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RouteSet {
	// Hostname -> []routes, sorted so that route matching can do a linear traversal
	inner: hashbrown::HashMap<HostnameMatch, Vec<SingleRouteMatch>>,
	// All routes
	all: HashMap<RouteKey, Arc<Route>>,
}

impl serde::Serialize for RouteSet {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.all.serialize(serializer)
	}
}

impl RouteSet {
	pub fn from_list(l: Vec<Route>) -> RouteSet {
		let mut rs = RouteSet::default();
		for ls in l.into_iter() {
			rs.insert(ls);
		}
		rs
	}

	pub fn get_hostname(
		&self,
		hnm: &HostnameMatchRef,
	) -> impl Iterator<Item = (Arc<Route>, &RouteMatch)> {
		self.inner.get(hnm).into_iter().flatten().flat_map(|rl| {
			self
				.all
				.get(&rl.key)
				.map(|r| (r.clone(), r.matches.get(rl.index).expect("corrupted state")))
		})
	}

	pub fn insert(&mut self, r: Route) {
		if self.all.contains_key(&r.key) {
			self.remove(&r.key);
		}
		let r = Arc::new(r);
		// Insert the route into all HashMap first so it's available during binary search
		self.all.insert(r.key.clone(), r.clone());

		for hostname_match in Self::hostname_matchers(&r) {
			let v = self.inner.entry(hostname_match).or_default();
			for (idx, m) in r.matches.iter().enumerate() {
				let to_insert = v.binary_search_by(|existing| {
					let have = self.all.get(&existing.key).expect("corrupted state");
					let have_match = have.matches.get(existing.index).expect("corrupted state");

					cmp::Ordering::reverse(Self::compare_route(
						(m, &r.key),
						(have_match, &existing.key),
					))
				});
				let insert_idx = to_insert.unwrap_or_else(|pos| pos);
				v.insert(
					insert_idx,
					SingleRouteMatch {
						key: r.key.clone(),
						index: idx,
					},
				);
			}
		}
	}

	fn compare_route(a: (&RouteMatch, &RouteKey), b: (&RouteMatch, &RouteKey)) -> Ordering {
		let (a, a_key) = a;
		let (b, b_key) = b;
		// Compare RouteMatch according to Gateway API sorting requirements
		// 1. Path match type (Exact > PathPrefix > Regex)
		let path_rank1 = get_path_rank(&a.path);
		let path_rank2 = get_path_rank(&b.path);
		if path_rank1 != path_rank2 {
			return cmp::Ordering::reverse(path_rank1.cmp(&path_rank2));
		}
		// 2. Path length (longer paths first)
		let path_len1 = get_path_length(&a.path);
		let path_len2 = get_path_length(&b.path);
		if path_len1 != path_len2 {
			return cmp::Ordering::reverse(path_len1.cmp(&path_len2)); // Reverse order for longer first
		}
		// 3. Method match (routes with method matches first)
		let method1 = a.method.is_some();
		let method2 = b.method.is_some();
		if method1 != method2 {
			return cmp::Ordering::reverse(method1.cmp(&method2));
		}
		// 4. Number of header matches (more headers first)
		let header_count1 = a.headers.len();
		let header_count2 = b.headers.len();
		if header_count1 != header_count2 {
			return cmp::Ordering::reverse(header_count1.cmp(&header_count2));
		}
		// 5. Number of query matches (more query params first)
		let query_count1 = a.query.len();
		let query_count2 = b.query.len();
		if query_count1 != query_count2 {
			return cmp::Ordering::reverse(query_count1.cmp(&query_count2));
		}
		// Finally, by order in the route list. This is the tie-breaker
		a_key.cmp(b_key)
	}

	pub fn contains(&self, key: &RouteKey) -> bool {
		self.all.contains_key(key)
	}

	pub fn remove(&mut self, key: &RouteKey) {
		let Some(old_route) = self.all.remove(key) else {
			return;
		};

		for hostname_match in Self::hostname_matchers(&old_route) {
			let entry = self
				.inner
				.entry(hostname_match)
				.and_modify(|v| v.retain(|r| &r.key != key));
			match entry {
				hashbrown::hash_map::Entry::Occupied(v) => {
					if v.get().is_empty() {
						v.remove();
					}
				},
				hashbrown::hash_map::Entry::Vacant(_) => {},
			}
		}
	}

	fn hostname_matchers(r: &Route) -> Vec<HostnameMatch> {
		if r.hostnames.is_empty() {
			vec![HostnameMatch::None]
		} else {
			r.hostnames
				.iter()
				.map(|h| HostnameMatch::from(h.clone()))
				.collect()
		}
	}

	pub fn is_empty(&self) -> bool {
		self.inner.is_empty()
	}

	pub fn iter(&self) -> impl Iterator<Item = &Arc<Route>> {
		self.all.values()
	}
}

#[derive(Debug, Clone, Default)]
pub struct TCPRouteSet {
	// Hostname -> []routes, sorted so that route matching can do a linear traversal
	inner: hashbrown::HashMap<HostnameMatch, Vec<RouteKey>>,
	// All routes
	all: HashMap<RouteKey, TCPRoute>,
}

impl serde::Serialize for TCPRouteSet {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		self.all.serialize(serializer)
	}
}

impl TCPRouteSet {
	pub fn from_list(l: Vec<TCPRoute>) -> Self {
		let mut rs = Self::default();
		for ls in l.into_iter() {
			rs.insert(ls);
		}
		rs
	}

	pub fn get_hostname(&self, hnm: &HostnameMatchRef) -> Option<&TCPRoute> {
		self
			.inner
			.get(hnm)
			.and_then(|r| r.first())
			.and_then(|rl| self.all.get(rl))
	}

	pub fn insert(&mut self, r: TCPRoute) {
		if self.all.contains_key(&r.key) {
			self.remove(&r.key);
		}
		// Insert the route into all HashMap first so it's available during binary search
		self.all.insert(r.key.clone(), r.clone());

		for hostname_match in Self::hostname_matchers(&r) {
			let v = self.inner.entry(hostname_match).or_default();
			let to_insert = v.binary_search_by(|existing| {
				let _have = self.all.get(existing).expect("corrupted state");
				// TODO: not sure that is right
				Ordering::reverse(r.key.cmp(existing))
			});
			let insert_idx = to_insert.unwrap_or_else(|pos| pos);
			v.insert(insert_idx, r.key.clone());
		}
	}

	pub fn contains(&self, key: &RouteKey) -> bool {
		self.all.contains_key(key)
	}

	pub fn remove(&mut self, key: &RouteKey) {
		let Some(old_route) = self.all.remove(key) else {
			return;
		};

		for hostname_match in Self::hostname_matchers(&old_route) {
			let entry = self
				.inner
				.entry(hostname_match)
				.and_modify(|v| v.retain(|r| r != key));
			match entry {
				hashbrown::hash_map::Entry::Occupied(v) => {
					if v.get().is_empty() {
						v.remove();
					}
				},
				hashbrown::hash_map::Entry::Vacant(_) => {},
			}
		}
	}

	fn hostname_matchers(r: &TCPRoute) -> Vec<HostnameMatch> {
		if r.hostnames.is_empty() {
			vec![HostnameMatch::None]
		} else {
			r.hostnames
				.iter()
				.map(|h| HostnameMatch::from(h.clone()))
				.collect()
		}
	}

	pub fn is_empty(&self) -> bool {
		self.inner.is_empty()
	}
}

// Helper functions for RouteMatch comparison
fn get_path_rank(path: &PathMatch) -> i32 {
	match path {
		// Best match: exact
		PathMatch::Exact(_) => 3,
		// Prefix/Regex -- we will defer to the length
		PathMatch::PathPrefix(_) => 2,
		PathMatch::Regex(_, _) => 2,
	}
}

fn get_path_length(path: &PathMatch) -> usize {
	match path {
		PathMatch::Exact(s) => s.len(),
		PathMatch::PathPrefix(s) => s.len(),
		PathMatch::Regex(_, l) => *l,
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, serde::Serialize)]
pub enum IpFamily {
	Dual,
	IPv4,
	IPv6,
}

pub type PolicyKey = Strng;
pub type BackendKey = Strng;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetedPolicy {
	pub key: PolicyKey,
	pub name: Option<TypedResourceName>,
	pub target: PolicyTarget,
	pub policy: PolicyType,
}

/// Configuration for dynamic tracing policy
#[apply(schema!)]
pub struct TracingConfig {
	#[serde(flatten)]
	pub provider_backend: SimpleBackendReference,
	/// Span attributes to add, keyed by attribute name.
	#[serde(default)]
	pub attributes: OrderedStringMap<Arc<cel::Expression>>,
	/// Resource attributes to add to the tracer provider (OTel `Resource`).
	/// This can be used to set things like `service.name` dynamically.
	#[serde(default)]
	pub resources: OrderedStringMap<Arc<cel::Expression>>,
	/// Attribute keys to remove from the emitted span attributes.
	///
	/// This is applied before `attributes` are evaluated/added, so it can be used to drop
	/// default attributes or avoid duplication.
	#[serde(default)]
	pub remove: Vec<String>,
	/// Optional per-policy override for random sampling. If set, overrides global config for
	/// requests that use this frontend policy.
	#[serde(default, deserialize_with = "deserialize_sampling_expr_opt")]
	pub random_sampling: Option<Arc<cel::Expression>>,
	/// Optional per-policy override for client sampling. If set, overrides global config for
	/// requests that use this frontend policy.
	#[serde(default, deserialize_with = "deserialize_sampling_expr_opt")]
	pub client_sampling: Option<Arc<cel::Expression>>,
	// OTLP path. Default is /v1/traces
	#[serde(default = "default_otlp_path")]
	pub path: String,
	// protocol specifies the OTLP protocol variant to use. Default is HTTP
	#[serde(default)]
	pub protocol: TracingProtocol,
}

fn default_otlp_path() -> String {
	"/v1/traces".to_string()
}

fn deserialize_sampling_expr_opt<'de, D>(
	deserializer: D,
) -> Result<Option<Arc<cel::Expression>>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	let v = Option::<crate::StringBoolFloat>::deserialize(deserializer)?;
	v.map(|v| cel::Expression::new_strict(&v.0))
		.transpose()
		.map(|o| o.map(Arc::new))
		.map_err(|e| serde::de::Error::custom(e.to_string()))
}

#[derive(serde::Serialize, serde::Deserialize, Default, Copy, Eq, PartialEq, Clone, Debug)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(crate::JsonSchema))]
pub enum TracingProtocol {
	#[default]
	Http,
	Grpc,
}

/// TracingPolicy holds both the configuration and the compiled OpenTelemetry tracer
#[derive(Clone, Debug)]
pub struct TracingPolicy {
	pub config: TracingConfig,
	/// CEL fields used by the tracer for span attributes. Stored so we can lazily
	/// create the tracer at first use with the correct attribute set.
	pub fields: Arc<crate::telemetry::log::LoggingFields>,
	/// Lazily initialized tracer. Created on first access in the dataplane
	/// using a PolicyClient so that backend routing and auth can be applied.
	pub tracer: once_cell::sync::OnceCell<Arc<crate::telemetry::trc::Tracer>>,
}

impl serde::Serialize for TracingPolicy {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.config.serialize(serializer)
	}
}

impl TracingPolicy {
	pub fn get_or_init(
		&self,
		policy_client: crate::proxy::httpproxy::PolicyClient,
	) -> anyhow::Result<&Arc<crate::telemetry::trc::Tracer>> {
		self.tracer.get_or_try_init(|| {
			let tracer = crate::telemetry::trc::Tracer::create_tracer_from_config_with_client(
				&self.config,
				self.fields.clone(),
				policy_client,
			)?;
			Ok(Arc::new(tracer))
		})
	}
}

impl From<BackendPolicy> for PolicyType {
	fn from(value: BackendPolicy) -> Self {
		Self::Backend(value)
	}
}

impl From<FrontendPolicy> for PolicyType {
	fn from(value: FrontendPolicy) -> Self {
		Self::Frontend(value)
	}
}

impl From<TrafficPolicy> for PolicyType {
	fn from(value: TrafficPolicy) -> Self {
		// Default to route for simplicity.
		(value, PolicyPhase::Route).into()
	}
}
impl From<(TrafficPolicy, PolicyPhase)> for PolicyType {
	fn from((p, phase): (TrafficPolicy, PolicyPhase)) -> Self {
		Self::Traffic(PhasedTrafficPolicy { phase, policy: p })
	}
}

#[apply(schema!)]
#[derive(Copy, Default, Eq, PartialEq)]
pub enum PolicyPhase {
	#[default]
	Route,
	Gateway,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhasedTrafficPolicy {
	pub phase: PolicyPhase,
	#[serde(flatten)]
	pub policy: TrafficPolicy,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PolicyType {
	Frontend(FrontendPolicy),
	Traffic(PhasedTrafficPolicy),
	Backend(BackendPolicy),
}

impl PolicyType {
	pub fn as_traffic_gateway_phase(&self) -> Option<&TrafficPolicy> {
		match self {
			PolicyType::Traffic(t) if t.phase == PolicyPhase::Gateway => Some(&t.policy),
			_ => None,
		}
	}
	pub fn as_traffic_route_phase(&self) -> Option<&TrafficPolicy> {
		match self {
			PolicyType::Traffic(t) if t.phase == PolicyPhase::Route => Some(&t.policy),
			_ => None,
		}
	}
	pub fn as_backend(&self) -> Option<&BackendPolicy> {
		match self {
			PolicyType::Backend(t) => Some(t),
			_ => None,
		}
	}
	pub fn as_frontend(&self) -> Option<&FrontendPolicy> {
		match self {
			PolicyType::Frontend(t) => Some(t),
			_ => None,
		}
	}
}

pub type RouteTarget = RouteName;

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
pub enum PolicyTarget {
	Gateway(ListenerTarget),
	Route(RouteTarget),
	Backend(BackendTarget),
}

impl Equivalent<PolicyTarget> for PolicyTargetRef<'_> {
	fn equivalent(&self, key: &PolicyTarget) -> bool {
		self == &PolicyTargetRef::from(key)
	}
}

#[derive(Hash, Eq, PartialEq)]
pub enum PolicyTargetRef<'a> {
	Gateway {
		gateway_name: &'a str,
		gateway_namespace: &'a str,
		listener_name: Option<&'a str>,
	},
	Route {
		name: &'a str,
		namespace: &'a str,
		rule_name: Option<&'a str>,
		kind: Option<&'a str>,
	},
	Backend(BackendTargetRef<'a>),
}

impl<'a> From<&'a PolicyTarget> for PolicyTargetRef<'a> {
	fn from(value: &'a PolicyTarget) -> Self {
		match value {
			PolicyTarget::Gateway(v) => PolicyTargetRef::Gateway {
				gateway_name: &v.gateway_name,
				gateway_namespace: v.gateway_namespace.as_ref(),
				listener_name: v.listener_name.as_deref(),
			},
			PolicyTarget::Route(v) => PolicyTargetRef::Route {
				name: &v.name,
				namespace: v.namespace.as_ref(),
				rule_name: v.rule_name.as_deref(),
				kind: v.kind.as_deref(),
			},
			PolicyTarget::Backend(v) => PolicyTargetRef::Backend(v.into()),
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendPolicy {
	HTTP(frontend::HTTP),
	TLS(frontend::TLS),
	TCP(frontend::TCP),
	AccessLog(frontend::LoggingPolicy),
	Tracing(Arc<TracingPolicy>),
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrafficPolicy {
	Timeout(timeout::Policy),
	Retry(retry::Policy),
	#[serde(rename = "ai")]
	AI(Arc<llm::Policy>),
	Authorization(Authorization),
	LocalRateLimit(Vec<crate::http::localratelimit::RateLimit>),
	RemoteRateLimit(remoteratelimit::RemoteRateLimit),
	ExtAuthz(ext_authz::ExtAuthz),
	ExtProc(ext_proc::ExtProc),
	JwtAuth(crate::http::jwt::Jwt),
	BasicAuth(crate::http::basicauth::BasicAuthentication),
	APIKey(crate::http::apikey::APIKeyAuthentication),
	Transformation(crate::http::transformation_cel::Transformation),
	Csrf(crate::http::csrf::Csrf),

	RequestHeaderModifier(filters::HeaderModifier),
	ResponseHeaderModifier(filters::HeaderModifier),
	RequestRedirect(filters::RequestRedirect),
	UrlRewrite(filters::UrlRewrite),
	HostRewrite(agent::HostRedirectOverride),
	RequestMirror(Vec<filters::RequestMirror>),
	DirectResponse(filters::DirectResponse),
	#[serde(rename = "cors")]
	CORS(http::cors::Cors),
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendPolicy {
	McpAuthorization(McpAuthorization),
	McpAuthentication(McpAuthentication),
	A2a(A2aPolicy),
	#[serde(rename = "http")]
	HTTP(backend::HTTP),
	#[serde(rename = "tcp")]
	TCP(backend::TCP),
	#[serde(rename = "backendTLS")]
	BackendTLS(http::backendtls::BackendTLS),
	BackendAuth(BackendAuth),
	InferenceRouting(ext_proc::InferenceRouting),
	#[serde(rename = "ai")]
	AI(Arc<llm::Policy>),
	SessionPersistence(http::sessionpersistence::Policy),

	RequestHeaderModifier(filters::HeaderModifier),
	ResponseHeaderModifier(filters::HeaderModifier),
	RequestRedirect(filters::RequestRedirect),
	RequestMirror(Vec<filters::RequestMirror>),
}

#[apply(schema!)]
pub struct A2aPolicy {}

#[apply(schema!)]
pub struct Authorization(pub RuleSet);

// Do not use schema! as it will reject the `extra` field
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ResourceMetadata {
	#[serde(flatten)]
	pub extra: BTreeMap<String, Value>,
}

impl ResourceMetadata {
	/// Build RFC-compliant JSON for the protected resource metadata.
	///
	/// - Defaults computed `resource` and `authorization_servers`.
	/// - Converts any additional config keys from camelCase to snake_case.
	/// - Adds MCP-specific fields used by the gateway.
	pub fn to_rfc_json(&self, resource_uri: String, issuer: String) -> Value {
		let mut map = serde_json::Map::new();

		// Computed fields. User can override them if they explicitly configure them.
		map.insert("resource".into(), Value::String(resource_uri));
		map.insert(
			"authorization_servers".into(),
			Value::Array(vec![Value::String(issuer)]),
		);
		// MCP-specific additions
		map.insert(
			"mcp_protocol_version".into(),
			Value::String("2025-06-18".into()),
		);
		map.insert("resource_type".into(), Value::String("mcp-server".into()));

		// Copy user-provided extra keys, converting to snake_case
		for (key, value) in &self.extra {
			let snake = key.to_snake_case();
			map.insert(snake, value.clone());
		}

		Value::Object(map)
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpAuthentication {
	pub issuer: String,
	pub audiences: Vec<String>,
	pub provider: Option<McpIDP>,
	pub resource_metadata: ResourceMetadata,
	pub jwt_validator: Arc<crate::http::jwt::Jwt>,
	pub mode: McpAuthenticationMode,
}

#[apply(schema_enum!)]
#[derive(Default)]
pub enum McpAuthenticationMode {
	/// A valid token, issued by a configured issuer, must be present.
	/// This is the default option.
	#[default]
	Strict,
	/// If a token exists, validate it.
	/// Warning: this allows requests without a JWT token! Additionally, 401 errors will not be returned,
	/// which will not trigger clients to initiate an oauth flow.
	Optional,
	/// Requests are never rejected. This is useful for usage of claims in later steps (authorization, logging, etc).
	/// Warning: this allows requests without a JWT token! Additionally, 401 errors will not be returned,
	/// which will not trigger clients to initiate an oauth flow.
	Permissive,
}

impl From<McpAuthenticationMode> for crate::http::jwt::Mode {
	fn from(value: McpAuthenticationMode) -> crate::http::jwt::Mode {
		match value {
			McpAuthenticationMode::Strict => crate::http::jwt::Mode::Strict,
			McpAuthenticationMode::Optional => crate::http::jwt::Mode::Optional,
			McpAuthenticationMode::Permissive => crate::http::jwt::Mode::Permissive,
		}
	}
}

// Non-xds config for MCP authentication
#[apply(schema_de!)]
pub struct LocalMcpAuthentication {
	pub issuer: String,
	pub audiences: Vec<String>,
	pub provider: Option<McpIDP>,
	pub resource_metadata: ResourceMetadata,
	pub jwks: FileInlineOrRemote,
	#[serde(default)]
	pub mode: McpAuthenticationMode,
	#[serde(default)]
	pub jwt_validation_options: http::jwt::JWTValidationOptions,
}

impl LocalMcpAuthentication {
	pub fn as_jwt(&self) -> anyhow::Result<http::jwt::LocalJwtConfig> {
		let jwks = match &self.jwks {
			FileInlineOrRemote::Remote { url } => FileInlineOrRemote::Remote {
				url: if !url.to_string().is_empty() {
					url.clone()
				} else {
					match &self.provider {
						None | Some(McpIDP::Auth0 { .. }) => {
							format!("{}/.well-known/jwks.json", self.issuer).parse()?
						},
						Some(McpIDP::Keycloak { .. }) => {
							format!("{}/protocol/openid-connect/certs", self.issuer).parse()?
						},
					}
				},
			},
			FileInlineOrRemote::Inline(_) | FileInlineOrRemote::File { .. } => self.jwks.clone(),
		};

		Ok(http::jwt::LocalJwtConfig::Single {
			mode: self.mode.into(),
			issuer: self.issuer.clone(),
			audiences: Some(self.audiences.clone()),
			jwks,
			jwt_validation_options: self.jwt_validation_options.clone(),
		})
	}

	/// Translate the local (file/env) config into a runtime `McpAuthentication` with a ready validator.
	pub async fn translate(
		&self,
		client: crate::client::Client,
	) -> anyhow::Result<McpAuthentication> {
		let jwt_cfg = self.as_jwt()?;
		let jwt = jwt_cfg.try_into(client).await?;
		Ok(McpAuthentication {
			issuer: self.issuer.clone(),
			audiences: self.audiences.clone(),
			provider: self.provider.clone(),
			resource_metadata: self.resource_metadata.clone(),
			jwt_validator: Arc::new(jwt),
			mode: self.mode,
		})
	}
}

#[apply(schema!)]
pub enum McpIDP {
	Auth0 {},
	Keycloak {},
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "String"))]
pub enum Target {
	Address(SocketAddr),
	Hostname(Strng, u16),
	/// Unix domain socket path (e.g., "unix:/path/to/socket")
	UnixSocket(PathBuf),
}

impl<'de> serde::Deserialize<'de> for Target {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		serdes::de_parse(deserializer)
	}
}

impl serde::Serialize for Target {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		serializer.serialize_str(&self.to_string())
	}
}

impl TryFrom<(&str, u16)> for Target {
	type Error = anyhow::Error;

	fn try_from((host, port): (&str, u16)) -> Result<Self, Self::Error> {
		match host.parse::<IpAddr>() {
			Ok(target) => Ok(Target::Address(SocketAddr::new(target, port))),
			Err(_) => Ok(Target::Hostname(host.into(), port)),
		}
	}
}

impl TryFrom<&str> for Target {
	type Error = anyhow::Error;

	fn try_from(hostport: &str) -> Result<Self, Self::Error> {
		// Check for unix socket prefix
		if let Some(path) = hostport.strip_prefix("unix:") {
			return Ok(Target::UnixSocket(PathBuf::from(path)));
		}
		let Some((host, port)) = hostport.split_once(":") else {
			anyhow::bail!("invalid host:port: {hostport}");
		};
		let port: u16 = port.parse()?;
		(host, port).try_into()
	}
}

impl Display for Target {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let str = match self {
			Target::Address(addr) => addr.to_string(),
			Target::Hostname(hostname, port) => format!("{hostname}:{port}"),
			Target::UnixSocket(path) => format!("unix:{}", path.display()),
		};
		write!(f, "{str}")
	}
}

impl Target {
	pub fn hostport(&self) -> String {
		match self {
			Target::Address(addr) => addr.to_string(),
			Target::Hostname(hostname, port) => format!("{hostname}:{port}"),
			Target::UnixSocket(path) => path
				.file_name()
				.and_then(|os| os.to_str())
				.unwrap_or_default()
				.to_string(),
		}
	}
}

#[apply(schema!)]
pub struct KeepaliveConfig {
	#[serde(default = "defaults::always_true")]
	pub enabled: bool,
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(default = "defaults::keepalive_time")]
	pub time: Duration,
	#[serde(with = "serde_dur")]
	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	#[serde(default = "defaults::keepalive_interval")]
	pub interval: Duration,
	#[serde(default = "defaults::keepalive_retries")]
	pub retries: u32,
}

impl Default for KeepaliveConfig {
	fn default() -> Self {
		KeepaliveConfig {
			enabled: true,
			time: defaults::keepalive_time(),
			interval: defaults::keepalive_interval(),
			retries: defaults::keepalive_retries(),
		}
	}
}

pub mod defaults {
	use std::time::Duration;

	pub fn always_true() -> bool {
		true
	}
	pub fn keepalive_retries() -> u32 {
		9
	}
	pub fn keepalive_interval() -> Duration {
		Duration::from_secs(180)
	}
	pub fn keepalive_time() -> Duration {
		Duration::from_secs(180)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn route_match(path: &'static str) -> RouteMatch {
		RouteMatch {
			headers: vec![],
			path: PathMatch::PathPrefix(strng::new(path)),
			method: None,
			query: vec![],
		}
	}

	fn route(key: &'static str, hostnames: Vec<&'static str>, matches: Vec<RouteMatch>) -> Route {
		Route {
			key: strng::new(key),
			name: RouteName::default(),
			hostnames: hostnames.into_iter().map(strng::new).collect(),
			matches,
			backends: vec![],
			inline_policies: vec![],
		}
	}

	fn tcp_route(key: &'static str, hostnames: Vec<&'static str>) -> TCPRoute {
		TCPRoute {
			key: strng::new(key),
			name: RouteName::default(),
			hostnames: hostnames.into_iter().map(strng::new).collect(),
			backends: vec![],
		}
	}

	#[test]
	fn test_backend_type_categorization() {
		let opaque_backend = Backend::Opaque(
			ResourceName::new(strng::new("test-opaque"), strng::new("ns")),
			Target::Hostname(strng::new("example.com"), 443),
		);
		assert_eq!(opaque_backend.backend_type(), cel::BackendType::Static);
		assert_eq!(
			opaque_backend.backend_info().backend_type,
			cel::BackendType::Static
		);

		let invalid_backend = Backend::Invalid;
		assert_eq!(invalid_backend.backend_type(), cel::BackendType::Unknown);
		assert_eq!(
			invalid_backend.backend_info().backend_type,
			cel::BackendType::Unknown
		);

		let info = opaque_backend.backend_info();
		assert_eq!(info.backend_name, strng::new("ns/test-opaque"));
	}

	#[test]
	fn test_parse_key_ec_p256() {
		let ec_key = b"-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIGfhD3tZlZOmw7LfyyERnPCyOnzmqiy1VcwiK36ro1H5oAoGCCqGSM49
AwEHoUQDQgAEwWSdCtU7tQGYtpNpJXSB5VN4yT1lRXzHh8UOgWWqiYXX1WYHk8vf
63XQuFFo4YbnXLIPdRxfxk9HzwyPw8jW8Q==
-----END EC PRIVATE KEY-----";

		let result = parse_key(ec_key);
		assert!(result.is_ok());

		let key = result.unwrap();
		match key {
			PrivateKeyDer::Sec1(_) => {}, // Expected
			_ => panic!("Expected SEC1 (EC) private key format"),
		}
	}

	#[test]
	fn test_parse_key_ec_p384() {
		let ec_key = b"-----BEGIN EC PRIVATE KEY-----
MIGkAgEBBDDLaVsYgpuTvciGqF9ULn07Kk9k9bxvZxqMFQX3VIccWAMhP3qlKC9O
xK4lPQIqDnGgBwYFK4EEACKhZANiAASK2hFgrQdhSnKMTHUc0Kf42kwjAIvv0Nds
z766bcs7vNyDqYpw7Gtr5weUGnl8M9h6BpONpZIS9RECMPTdfsLmYqlX0DGsMR3v
L/VtP/WipvzV+9ejgYQwt0cOKYYCoSc=
-----END EC PRIVATE KEY-----";

		let result = parse_key(ec_key);
		assert!(result.is_ok());

		let key = result.unwrap();
		match key {
			PrivateKeyDer::Sec1(_) => {}, // Expected
			_ => panic!("Expected SEC1 (EC) private key format"),
		}
	}

	#[test]
	fn test_parse_key_pkcs8() {
		// Test existing PKCS8 support still works
		let pkcs8_key = b"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg7oRJ3/tWjzNRdSXj
k2kj5FhI/GKfGpvAJbDe6A4VlzuhRANCAASTGTFE0FdYwKqcaUEZ3VhqKlpZLjY/
SGjfUH8wjCgRLFmKGfZSFZFh1xN9M5Bq6v1P6kNqW7nM7oA4VJWqKp5W
-----END PRIVATE KEY-----";

		let result = parse_key(pkcs8_key);
		assert!(result.is_ok());

		let key = result.unwrap();
		match key {
			PrivateKeyDer::Pkcs8(_) => {}, // Expected
			_ => panic!("Expected PKCS8 private key format"),
		}
	}

	#[test]
	fn test_parse_key_invalid() {
		let invalid_key = b"-----BEGIN INVALID KEY-----
InvalidKeyData
-----END INVALID KEY-----";

		let result = parse_key(invalid_key);
		assert!(result.is_err());
		// Check for actual error message that rustls_pemfile returns
		let error_msg = result.unwrap_err().to_string();
		assert!(
			error_msg.contains("failed to fill whole buffer")
				|| error_msg.contains("no key")
				|| error_msg.contains("unsupported key")
		);
	}

	#[test]
	fn test_parse_key_empty() {
		let empty_key = b"";
		let result = parse_key(empty_key);
		assert!(result.is_err());
	}

	#[test]
	fn test_target_unix_socket_parse() {
		// Test parsing a Unix socket path
		let target = Target::try_from("unix:/var/run/test.sock").unwrap();
		assert!(
			matches!(target, Target::UnixSocket(ref path) if path == std::path::Path::new("/var/run/test.sock"))
		);
	}

	#[test]
	fn test_target_unix_socket_display() {
		// Test Display implementation for UnixSocket
		let target = Target::UnixSocket(PathBuf::from("/var/run/test.sock"));
		assert_eq!(target.to_string(), "unix:/var/run/test.sock");
	}

	#[test]
	fn test_target_unix_socket_roundtrip() {
		// Test that parsing and display are consistent
		let original = "unix:/tmp/my-socket.sock";
		let target = Target::try_from(original).unwrap();
		assert_eq!(target.to_string(), original);
	}

	#[test]
	fn test_target_address_still_works() {
		// Ensure regular host:port still works
		let target = Target::try_from("127.0.0.1:8080").unwrap();
		assert!(matches!(target, Target::Address(_)));
	}

	#[test]
	fn test_target_hostname_still_works() {
		// Ensure hostname:port still works
		let target = Target::try_from("example.com:443").unwrap();
		assert!(matches!(target, Target::Hostname(h, 443) if h.as_str() == "example.com"));
	}

	#[test]
	fn test_all_matches_subdomain() {
		let matches: Vec<_> = HostnameMatch::all_matches("api.example.com").collect();

		assert_eq!(matches.len(), 4);
		assert_eq!(matches[0], HostnameMatchRef::Exact("api.example.com"));
		assert_eq!(matches[1], HostnameMatchRef::Wildcard("example.com"));
		assert_eq!(matches[2], HostnameMatchRef::Wildcard("com"));
		assert_eq!(matches[3], HostnameMatchRef::None);

		let matches: Vec<_> = HostnameMatch::all_matches("*.example.com").collect();

		assert_eq!(matches.len(), 3);
		assert_eq!(matches[0], HostnameMatchRef::Wildcard("example.com"));
		assert_eq!(matches[1], HostnameMatchRef::Wildcard("com"));
		assert_eq!(matches[2], HostnameMatchRef::None);

		let matches: Vec<_> = HostnameMatch::all_matches("localhost").collect();

		assert_eq!(matches.len(), 2);
		assert_eq!(matches[0], HostnameMatchRef::Exact("localhost"));
		assert_eq!(matches[1], HostnameMatchRef::None);
	}

	#[test]
	fn test_route_set_iter() {
		// Create test routes with unique keys
		let route1 = route("route-1", vec![], vec![]);
		let route2 = route("route-2", vec![], vec![]);
		let route3 = route("route-3", vec![], vec![]);

		// Build RouteSet
		let route_set = RouteSet::from_list(vec![route1, route2, route3]);

		// Call iter() and collect keys
		let keys: std::collections::HashSet<_> = route_set.iter().map(|r| r.key.clone()).collect();

		// Verify all routes are returned
		assert_eq!(keys.len(), 3);
		assert!(keys.contains(&strng::new("route-1")));
		assert!(keys.contains(&strng::new("route-2")));
		assert!(keys.contains(&strng::new("route-3")));
	}

	#[test]
	fn test_route_set_insert_upsert_replaces_match_indexes() {
		let mut route_set = RouteSet::default();
		route_set.insert(route(
			"route-1",
			vec![],
			vec![route_match("/first"), route_match("/second")],
		));
		route_set.insert(route("route-1", vec![], vec![route_match("/first")]));

		let got: Vec<_> = route_set.get_hostname(&HostnameMatchRef::None).collect();
		assert_eq!(got.len(), 1);
		assert_eq!(got[0].0.key, strng::new("route-1"));
		match &got[0].1.path {
			PathMatch::PathPrefix(path) => assert_eq!(path.as_str(), "/first"),
			_ => panic!("expected PathPrefix match"),
		}
	}

	#[test]
	fn test_route_set_insert_upsert_cleans_old_hostname_entries() {
		let mut route_set = RouteSet::default();
		route_set.insert(route(
			"route-1",
			vec!["old.example.com"],
			vec![route_match("/old")],
		));
		route_set.insert(route(
			"route-1",
			vec!["new.example.com"],
			vec![route_match("/new")],
		));
		route_set.remove(&strng::new("route-1"));
		route_set.insert(route(
			"route-2",
			vec!["old.example.com"],
			vec![route_match("/current")],
		));

		let got: Vec<_> = route_set
			.get_hostname(&HostnameMatchRef::Exact("old.example.com"))
			.collect();
		assert_eq!(got.len(), 1);
		assert_eq!(got[0].0.key, strng::new("route-2"));
	}

	#[test]
	fn test_tcp_route_set_insert_upsert_cleans_old_hostname_entries() {
		let mut route_set = TCPRouteSet::default();
		route_set.insert(tcp_route("tcp-1", vec!["old.example.com"]));
		route_set.insert(tcp_route("tcp-1", vec!["new.example.com"]));
		route_set.remove(&strng::new("tcp-1"));
		route_set.insert(tcp_route("tcp-2", vec!["old.example.com"]));

		let got = route_set
			.get_hostname(&HostnameMatchRef::Exact("old.example.com"))
			.expect("route should be present");
		assert_eq!(got.key, strng::new("tcp-2"));
	}

	#[test]
	fn test_local_mcp_authentication_default_jwt_validation_options() {
		let yaml = r#"
issuer: "https://example.com"
audiences: ["aud1"]
jwks: '{"keys":[]}'
resourceMetadata:
  mcpResourceUri: "mcp://test"
"#;
		let auth: LocalMcpAuthentication = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(
			auth.jwt_validation_options.required_claims,
			std::collections::HashSet::from(["exp".to_owned()]),
			"default required_claims should be [\"exp\"]"
		);
	}

	#[test]
	fn test_local_mcp_authentication_jwt_validation_options_present_but_required_claims_missing() {
		let yaml = r#"
issuer: "https://example.com"
audiences: ["aud1"]
jwks: '{"keys":[]}'
resourceMetadata:
  mcpResourceUri: "mcp://test"
jwtValidationOptions: {}
"#;
		let auth: LocalMcpAuthentication = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(
			auth.jwt_validation_options.required_claims,
			std::collections::HashSet::from(["exp".to_owned()]),
			"omitted requiredClaims should default to [\"exp\"]"
		);
	}

	#[test]
	fn test_local_mcp_authentication_with_empty_required_claims() {
		let yaml = r#"
issuer: "https://enterprise-idp.example.com"
audiences: ["enterprise-aud"]
jwks: '{"keys":[]}'
resourceMetadata:
  mcpResourceUri: "mcp://test"
jwtValidationOptions:
  requiredClaims: []
"#;
		let auth: LocalMcpAuthentication = serde_yaml::from_str(yaml).unwrap();
		assert!(
			auth.jwt_validation_options.required_claims.is_empty(),
			"required_claims should be empty"
		);
	}

	#[test]
	fn test_local_mcp_authentication_with_custom_required_claims() {
		let yaml = r#"
issuer: "https://enterprise-idp.example.com"
audiences: ["enterprise-aud"]
jwks: '{"keys":[]}'
resourceMetadata:
  mcpResourceUri: "mcp://test"
jwtValidationOptions:
  requiredClaims: ["exp", "nbf"]
"#;
		let auth: LocalMcpAuthentication = serde_yaml::from_str(yaml).unwrap();
		assert_eq!(
			auth.jwt_validation_options.required_claims,
			std::collections::HashSet::from(["exp".to_owned(), "nbf".to_owned()])
		);
	}

	#[test]
	fn test_local_mcp_authentication_as_jwt_propagates_jwt_validation_options() {
		let yaml = r#"
issuer: "https://enterprise-idp.example.com"
audiences: ["enterprise-aud"]
jwks: '{"keys":[]}'
resourceMetadata:
  mcpResourceUri: "mcp://test"
jwtValidationOptions:
  requiredClaims: []
"#;
		let auth: LocalMcpAuthentication = serde_yaml::from_str(yaml).unwrap();
		let jwt_config = auth.as_jwt().unwrap();

		match jwt_config {
			http::jwt::LocalJwtConfig::Single {
				jwt_validation_options,
				..
			} => {
				assert!(
					jwt_validation_options.required_claims.is_empty(),
					"jwt_validation_options should be propagated to LocalJwtConfig"
				);
			},
			_ => panic!("Expected LocalJwtConfig::Single"),
		}
	}
}
