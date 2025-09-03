use std::cmp;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
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
	HeaderName, HeaderValue, ext_authz, ext_proc, filters, remoteratelimit, retry, timeout,
};
use crate::mcp::rbac::McpAuthorization;
use crate::types::discovery::{NamespacedHostname, Service};
use crate::*;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bind {
	pub key: BindName,
	pub address: SocketAddr,
	pub listeners: ListenerSet,
}

pub type BindName = Strng;
pub type ListenerName = Strng;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listener {
	pub key: ListenerKey,
	// User facing name
	pub name: ListenerName,
	// User facing name
	pub gateway_name: GatewayName,
	/// Can be a wildcard
	pub hostname: Strng,
	pub protocol: ListenerProtocol,
	pub routes: RouteSet,
	pub tcp_routes: TCPRouteSet,
}

pub type GatewayName = Strng;

#[derive(Debug, Clone)]
pub struct TLSConfig {
	pub config: Arc<ServerConfig>,
}

impl serde::Serialize for TLSConfig {
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
	HTTP,
	HTTPS(TLSConfig),
	TLS(TLSConfig),
	TCP,
	HBONE,
}

impl ListenerProtocol {
	pub fn tls(&self) -> Option<Arc<rustls::ServerConfig>> {
		match self {
			ListenerProtocol::HTTPS(t) | ListenerProtocol::TLS(t) => Some(t.config.clone()),
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
	// User facing name of the route
	pub route_name: RouteName,
	// User facing name of the rule
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rule_name: Option<RouteRuleName>,
	/// Can be a wildcard
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub hostnames: Vec<Strng>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub matches: Vec<RouteMatch>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<RouteFilter>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub backends: Vec<RouteBackendReference>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub policies: Option<TrafficPolicy>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub inline_policies: Vec<Policy>,
}

pub type RouteKey = Strng;
pub type RouteName = Strng;
pub type RouteRuleName = Strng;

#[apply(schema_ser!)]
pub struct TCPRoute {
	// Internal name
	pub key: RouteKey,
	// User facing name of the route
	pub route_name: RouteName,
	// Can be a wildcard. Not applicable for TCP, only for TLS
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub hostnames: Vec<Strng>,
	// User facing name of the rule
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub rule_name: Option<RouteRuleName>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub backends: Vec<TCPRouteBackendReference>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct TCPRouteBackendReference {
	#[serde(default = "default_weight")]
	pub weight: usize,
	pub backend: SimpleBackendReference,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TCPRouteBackend {
	#[serde(default = "default_weight")]
	pub weight: usize,
	pub backend: SimpleBackend,
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
	pub name: HeaderName,
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RouteFilter {
	RequestHeaderModifier(filters::HeaderModifier),
	ResponseHeaderModifier(filters::HeaderModifier),
	RequestRedirect(filters::RequestRedirect),
	UrlRewrite(filters::UrlRewrite),
	RequestMirror(filters::RequestMirror),
	// TODO: xds support
	DirectResponse(filters::DirectResponse),
	#[serde(rename = "cors")]
	CORS(http::cors::Cors),
}

#[derive(Default, Eq, PartialEq)]
#[apply(schema!)]
pub struct TrafficPolicy {
	pub timeout: timeout::Policy,
	pub retry: Option<retry::Policy>,
}

#[apply(schema!)]
#[derive(Eq, PartialEq)]
pub enum HostRedirect {
	Full(Strng),
	Host(Strng),
	Port(NonZeroU16),
	Auto,
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
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<RouteFilter>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteBackend {
	#[serde(default = "default_weight")]
	pub weight: usize,
	#[serde(flatten)]
	pub backend: Backend,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub filters: Vec<RouteFilter>,
}

#[allow(unused)]
fn default_weight() -> usize {
	1
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
	Service(Arc<Service>, u16),
	#[serde(rename = "host", serialize_with = "serialize_backend_tuple")]
	Opaque(BackendName, Target), // Hostname or IP
	#[serde(rename = "mcp", serialize_with = "serialize_backend_tuple")]
	MCP(BackendName, McpBackend),
	#[serde(rename = "ai", serialize_with = "serialize_backend_tuple")]
	AI(BackendName, crate::llm::AIBackend),
	Dynamic {},
	Invalid,
}

pub fn serialize_backend_tuple<S: Serializer, T: serde::Serialize>(
	name: &BackendName,
	t: T,
	serializer: S,
) -> Result<S::Ok, S::Error> {
	#[derive(Debug, Clone, serde::Serialize)]
	#[serde(rename_all = "camelCase")]
	struct BackendTuple<'a, T: serde::Serialize> {
		name: &'a BackendName,
		target: &'a T,
	}
	BackendTuple { name, target: &t }.serialize(serializer)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendReference {
	Service { name: NamespacedHostname, port: u16 },
	Backend(BackendName),
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
	Opaque(BackendName, Target), // Hostname or IP
	Invalid,
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

#[apply(schema_ser!)]
pub enum SimpleBackendReference {
	Service { name: NamespacedHostname, port: u16 },
	Backend(BackendName), // Hostname or IP
	Invalid,
}

impl SimpleBackendReference {
	pub fn name(&self) -> BackendName {
		match self {
			SimpleBackendReference::Service { name, port } => {
				strng::format!("service/{}/{}:{port}", name.namespace, name.hostname)
			},
			SimpleBackendReference::Backend(name) => name.clone(),
			SimpleBackendReference::Invalid => strng::format!("invalid"),
		}
	}
}

impl SimpleBackend {
	pub fn hostport(&self) -> String {
		match self {
			SimpleBackend::Service(svc, port) => {
				format!("{}:{port}", svc.hostname)
			},
			SimpleBackend::Opaque(_, tgt) => tgt.to_string(),
			SimpleBackend::Invalid => "invalid".to_string(),
		}
	}
	pub fn name(&self) -> BackendName {
		match self {
			SimpleBackend::Service(svc, port) => {
				strng::format!("service/{}/{}:{port}", svc.namespace, svc.hostname)
			},
			SimpleBackend::Opaque(name, _) => name.clone(),
			SimpleBackend::Invalid => strng::format!("invalid"),
		}
	}
}

impl BackendReference {
	pub fn name(&self) -> BackendName {
		match self {
			BackendReference::Service { name, port } => {
				strng::format!("service/{}/{}:{port}", name.namespace, name.hostname)
			},
			BackendReference::Backend(name) => name.clone(),
			BackendReference::Invalid => strng::format!("invalid"),
		}
	}
}
impl Backend {
	pub fn name(&self) -> BackendName {
		match self {
			Backend::Service(svc, port) => {
				strng::format!("service/{}/{}:{port}", svc.namespace, svc.hostname)
			},
			Backend::Opaque(name, _) => name.clone(),
			Backend::MCP(name, _) => name.clone(),
			Backend::AI(name, _) => name.clone(),
			// TODO: give it a name
			Backend::Dynamic {} => strng::format!("dynamic"),
			Backend::Invalid => strng::format!("invalid"),
		}
	}
}

pub type BackendName = Strng;
pub type ServiceName = Strng;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct McpBackend {
	pub targets: Vec<Arc<McpTarget>>,
	pub stateful: bool,
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
	let schema: OpenAPI = yamlviajson::from_str(s.as_str()).map_err(serde::de::Error::custom)?;
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
	all: HashMap<RouteKey, Route>,
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

	pub fn get_hostname(&self, hnm: &HostnameMatch) -> impl Iterator<Item = (&Route, &RouteMatch)> {
		self.inner.get(hnm).into_iter().flatten().flat_map(|rl| {
			self
				.all
				.get(&rl.key)
				.map(|r| (r, r.matches.get(rl.index).expect("corrupted state")))
		})
	}

	pub fn insert(&mut self, r: Route) {
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

pub type PolicyName = Strng;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetedPolicy {
	pub name: PolicyName,
	pub target: PolicyTarget,
	pub policy: Policy,
}

#[derive(Hash, Eq, PartialEq)]
#[apply(schema!)]
pub enum PolicyTarget {
	Gateway(GatewayName),
	Listener(ListenerKey),
	Route(RouteName),
	RouteRule(RouteRuleName),
	// Note: Backend includes Service:port, this is used when we are *only* attaching to service
	Service(ServiceName),
	Backend(BackendName),
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Policy {
	// Supported targets: Backend, only when Backend type is MCP
	McpAuthorization(McpAuthorization),
	// Supported targets: Backend, only when Backend type is MCP
	McpAuthentication(McpAuthentication),
	// Support targets: Backend; single policy allowed
	A2a(A2aPolicy),
	// Supported targets: Backend; single policy allowed
	#[serde(rename = "backendTLS")]
	BackendTLS(http::backendtls::BackendTLS),
	// Supported targets: Backend; single policy allowed
	BackendAuth(BackendAuth),
	// Supported targets: Backend; single policy allowed
	InferenceRouting(ext_proc::InferenceRouting),

	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	#[serde(rename = "ai")]
	AI(Arc<llm::Policy>),
	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	Authorization(Authorization),
	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	LocalRateLimit(Vec<crate::http::localratelimit::RateLimit>),
	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	RemoteRateLimit(remoteratelimit::RemoteRateLimit),
	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	ExtAuthz(ext_authz::ExtAuthz),
	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	JwtAuth(crate::http::jwt::Jwt),
	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	// Supported targets: Gateway < Route < RouteRule; single policy allowed
	Transformation(crate::http::transformation_cel::Transformation),
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
	pub resource: String,
	#[serde(flatten)]
	pub extra: BTreeMap<String, Value>,
}

impl ResourceMetadata {
	/// Build RFC-compliant JSON for the protected resource metadata.
	///
	/// - Enforces computed `resource` and `authorization_servers`.
	/// - Converts any additional config keys from camelCase to snake_case.
	/// - Adds MCP-specific fields used by the gateway.
	pub fn to_rfc_json(&self, resource_uri: String, issuer: String) -> Value {
		let mut map = serde_json::Map::new();

		// Copy user-provided extra keys, converting to snake_case, while preventing overrides
		// of fields we compute.
		for (key, value) in &self.extra {
			let snake = key.to_snake_case();
			if snake == "resource" || snake == "authorization_servers" {
				continue;
			}
			map.insert(snake, value.clone());
		}

		// Computed fields
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

		Value::Object(map)
	}
}

#[apply(schema!)]
pub struct McpAuthentication {
	pub issuer: String,
	pub audience: String,
	pub jwks_url: String,
	pub provider: Option<McpIDP>,
	pub resource_metadata: ResourceMetadata,
}

impl McpAuthentication {
	pub fn as_jwt(&self) -> anyhow::Result<http::jwt::LocalJwtConfig> {
		Ok(http::jwt::LocalJwtConfig::Single {
			mode: http::jwt::Mode::Optional,
			issuer: self.issuer.clone(),
			audiences: vec![self.audience.clone()],
			jwks: FileInlineOrRemote::Remote {
				url: if !self.jwks_url.is_empty() {
					self.jwks_url.parse()?
				} else {
					match &self.provider {
						None | Some(McpIDP::Auth0 { .. }) => {
							format!("{}/.well-known/jwks.json", self.issuer).parse()?
						},
						Some(McpIDP::Keycloak { .. }) => {
							format!("{}/protocol/openid-connect/certs", self.issuer).parse()?
						},
						// Some(McpIDP::Keycloak { realm }) => format!("{}/realms/{realm}/protocol/openid-connect/certs", self.issuer).parse()?,
					}
				},
			},
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
		let Some((host, port)) = hostport.split_once(":") else {
			anyhow::bail!("invalid host:port: {}", hostport);
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
		};
		write!(f, "{str}")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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
}
