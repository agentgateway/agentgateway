use std::cmp;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;

use anyhow::anyhow;
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
use crate::types::discovery::{NamespacedHostname, Service};
use crate::types::local::SimpleLocalBackend;
use crate::types::{agent, backend, frontend};
use crate::*;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bind {
	pub key: BindKey,
	pub address: SocketAddr,
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

type Alpns = Vec<Vec<u8>>;

#[derive(Debug, Clone)]
pub struct ServerTLSConfig {
	config: Option<Arc<ServerConfig>>,
	per_alpn_config: Arc<RwLock<HashMap<Alpns, Arc<ServerConfig>>>>,
}

impl ServerTLSConfig {
	pub fn new(config: Arc<ServerConfig>) -> Self {
		Self {
			config: Some(config),
			per_alpn_config: Arc::new(Default::default()),
		}
	}
	/// new_invalid returns a ServerTLSConfig that always rejects connections
	pub fn new_invalid() -> Self {
		Self {
			config: None,
			per_alpn_config: Arc::new(Default::default()),
		}
	}
	/// config_for returns the appropriate config for the requested ALPN
	/// If none is return, it means the certificates were invalid.
	pub fn config_for(&self, alpns: Option<&[Vec<u8>]>) -> Option<Arc<ServerConfig>> {
		let config = self.config.clone()?;
		let Some(alpn) = alpns else {
			return Some(config);
		};
		{
			let reader = self.per_alpn_config.read().unwrap();
			if let Some(cached_config) = reader.get(alpn) {
				return Some(Arc::clone(cached_config));
			};
		}
		let mut writer = self.per_alpn_config.write().unwrap();
		if let Some(cached_config) = writer.get(alpn) {
			return Some(Arc::clone(cached_config));
		}
		let mut new_config = Arc::unwrap_or_clone(config);
		new_config.alpn_protocols = alpn.to_vec();
		let arc_config = Arc::new(new_config);

		writer.insert(alpn.to_vec(), Arc::clone(&arc_config));
		Some(arc_config)
	}
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
		alpns: Option<&[Vec<u8>]>,
	) -> Option<anyhow::Result<Arc<rustls::ServerConfig>>> {
		match self {
			ListenerProtocol::HTTPS(t) => Some(
				t.config_for(alpns)
					.ok_or_else(|| anyhow!("TLS certificate invalid")),
			),
			ListenerProtocol::TLS(t) => t.as_ref().map(|t| {
				t.config_for(alpns)
					.ok_or_else(|| anyhow!("TLS certificate invalid"))
			}),
			_ => None,
		}
	}
}

// Protocol of the entire bind. TODO: we should make this a property of the API
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, EncodeLabelValue)]
#[allow(non_camel_case_types)]
pub enum BindProtocol {
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
#[cfg_attr(test, derive(Default))]
pub struct RouteName {
	pub name: Strng,
	pub namespace: Strng,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rule_name: Option<Strng>,
}

impl RouteName {
	pub fn as_route_name(&self) -> Strng {
		strng::format!("{}/{}", self.namespace, self.name)
	}
	pub fn strip_route_rule_field(&self) -> RouteName {
		Self {
			name: self.name.clone(),
			namespace: self.namespace.clone(),
			rule_name: None,
		}
	}
}

#[apply(schema!)]
#[derive(Hash, Eq, PartialEq)]
#[cfg_attr(test, derive(Default))]
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

impl BackendTarget {
	pub fn strip_section(&self) -> BackendTarget {
		match self.clone() {
			BackendTarget::Backend {
				name, namespace, ..
			} => BackendTarget::Backend {
				name,
				namespace,
				section: None,
			},
			BackendTarget::Service {
				namespace,
				hostname,
				..
			} => BackendTarget::Service {
				namespace,
				hostname,
				port: None,
			},
			BackendTarget::Invalid => BackendTarget::Invalid,
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

impl SimpleBackendReference {
	// pub fn name(&self) -> BackendName {
	// 	match self {
	// 		SimpleBackendReference::Service { name, port } => {
	// 			strng::format!("service/{}/{}:{port}", name.namespace, name.hostname)
	// 		},
	// 		SimpleBackendReference::Backend(name) => name.clone(),
	// 		SimpleBackendReference::InlineBackend(t) => t.to_string().into(),
	// 		SimpleBackendReference::Invalid => strng::format!("invalid"),
	// 	}
	// }
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

	pub fn target(&self) -> BackendTarget {
		match self {
			SimpleBackend::Service(svc, port) => BackendTarget::Service {
				hostname: svc.hostname.clone(),
				namespace: svc.namespace.clone(),
				port: Some(*port),
			},
			SimpleBackend::Opaque(name, _) => BackendTarget::Backend {
				name: name.name.clone(),
				namespace: name.namespace.clone(),
				section: None,
			},
			SimpleBackend::Invalid => BackendTarget::Invalid,
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
	pub fn all_matches_or_none(
		hostname: Option<&str>,
	) -> Box<dyn Iterator<Item = HostnameMatch> + '_> {
		match hostname {
			None => Box::new(std::iter::once(HostnameMatch::None)),
			Some(h) => Box::new(Self::all_matches(h)),
		}
	}
	pub fn all_matches(hostname: &str) -> impl Iterator<Item = HostnameMatch> + '_ {
		Self::all_actual_matches(hostname).chain(std::iter::once(HostnameMatch::None))
	}
	fn all_actual_matches(hostname: &str) -> impl Iterator<Item = HostnameMatch> + '_ {
		let start = if hostname.starts_with("*.") {
			None
		} else {
			Some(HostnameMatch::Exact(hostname.into()))
		};
		// Build wildcards in reverse order by collecting parts and building from longest to shortest
		let parts: Vec<_> = hostname.split('.').skip(1).collect();
		let wildcards = (0..parts.len()).map(move |i| {
			let suffix = parts[i..].join(".");
			HostnameMatch::Wildcard(suffix.into())
		});
		start.into_iter().chain(wildcards)
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
	inner: HashMap<HostnameMatch, Vec<SingleRouteMatch>>,
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
		hnm: &HostnameMatch,
	) -> impl Iterator<Item = (Arc<Route>, &RouteMatch)> {
		self.inner.get(hnm).into_iter().flatten().flat_map(|rl| {
			self
				.all
				.get(&rl.key)
				.map(|r| (r.clone(), r.matches.get(rl.index).expect("corrupted state")))
		})
	}

	pub fn insert(&mut self, r: Route) {
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
				// TODO: replace old route
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
				Entry::Occupied(v) => {
					if v.get().is_empty() {
						v.remove();
					}
				},
				Entry::Vacant(_) => {},
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
}

#[derive(Debug, Clone, Default)]
pub struct TCPRouteSet {
	// Hostname -> []routes, sorted so that route matching can do a linear traversal
	inner: HashMap<HostnameMatch, Vec<RouteKey>>,
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

	pub fn get_hostname(&self, hnm: &HostnameMatch) -> Option<&TCPRoute> {
		self
			.inner
			.get(hnm)
			.and_then(|r| r.first())
			.and_then(|rl| self.all.get(rl))
	}

	pub fn insert(&mut self, r: TCPRoute) {
		// Insert the route into all HashMap first so it's available during binary search
		self.all.insert(r.key.clone(), r.clone());

		for hostname_match in Self::hostname_matchers(&r) {
			let v = self.inner.entry(hostname_match).or_default();
			let to_insert = v.binary_search_by(|existing| {
				let _have = self.all.get(existing).expect("corrupted state");
				// TODO: not sure that is right
				Ordering::reverse(r.key.cmp(existing))
			});
			// TODO: replace old route
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
				Entry::Occupied(v) => {
					if v.get().is_empty() {
						v.remove();
					}
				},
				Entry::Vacant(_) => {},
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FrontendPolicy {
	HTTP(frontend::HTTP),
	TLS(frontend::TLS),
	TCP(frontend::TCP),
	AccessLog(frontend::LoggingPolicy),
	Tracing(()),
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
	AI(Arc<llm::Policy>),

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
	pub mode: http::jwt::Mode,
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
	pub mode: http::jwt::Mode,
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
			mode: self.mode,
			issuer: self.issuer.clone(),
			audiences: Some(self.audiences.clone()),
			jwks,
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
			matches!(target, Target::UnixSocket(path) if path == PathBuf::from("/var/run/test.sock"))
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
}
