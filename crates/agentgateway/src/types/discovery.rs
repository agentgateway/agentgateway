use std::collections::HashMap;
use std::fmt;
use std::fmt::{Formatter, Write};
use std::hash::Hash;
use std::net::IpAddr;
use std::ops::Deref;
use std::str::FromStr;

use anyhow::anyhow;
use prometheus_client::encoding::{EncodeLabelValue, LabelValueEncoder};
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::loadbalancer;
use crate::types::proto::workload::load_balancing::Scope as XdsScope;
use crate::types::proto::workload::{
	GatewayAddress as XdsGatewayAddress, Port, PortList, Service as XdsService,
	Workload as XdsWorkload,
};
use crate::types::proto::{ProtoError, workload};
use crate::*;

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct NamespacedHostname {
	pub namespace: Strng,
	pub hostname: Strng,
}

impl FromStr for NamespacedHostname {
	type Err = ProtoError;

	fn from_str(value: &str) -> Result<Self, Self::Err> {
		let Some((namespace, hostname)) = value.split_once('/') else {
			return Err(ProtoError::NamespacedHostnameParse(value.to_string()));
		};
		Ok(Self {
			namespace: namespace.into(),
			hostname: hostname.into(),
		})
	}
}

// we need custom serde serialization since NamespacedHostname is keying maps
impl Serialize for NamespacedHostname {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.collect_str(&self)
	}
}

// we need custom serde deserialization because we have custom serialization
impl<'de> Deserialize<'de> for NamespacedHostname {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct NamespacedHostnameVisitor;

		impl Visitor<'_> for NamespacedHostnameVisitor {
			type Value = NamespacedHostname;

			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
				formatter.write_str("string for NamespacedHostname with format namespace/hostname")
			}

			fn visit_str<E>(self, value: &str) -> Result<NamespacedHostname, E>
			where
				E: serde::de::Error,
			{
				NamespacedHostname::from_str(value)
					.map_err(|_| serde::de::Error::invalid_value(serde::de::Unexpected::Str(value), &self))
			}
		}
		deserializer.deserialize_str(NamespacedHostnameVisitor)
	}
}

impl fmt::Display for NamespacedHostname {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}/{}", self.namespace, self.hostname)
	}
}

#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct NetworkAddress {
	pub network: Strng,
	pub address: IpAddr,
}

// we need custom serde serialization since NetworkAddress is keying maps
impl Serialize for NetworkAddress {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		serializer.collect_str(&self)
	}
}

impl fmt::Display for NetworkAddress {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.write_str(&self.network)?;
		f.write_char('/')?;
		f.write_str(&self.address.to_string())
	}
}

// we need custom serde deserialization because we have custom serialization
impl<'de> Deserialize<'de> for NetworkAddress {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct NetworkAddressVisitor;

		impl Visitor<'_> for NetworkAddressVisitor {
			type Value = NetworkAddress;

			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
				formatter.write_str("string for NetworkAddress with format network/IP")
			}

			fn visit_str<E>(self, value: &str) -> Result<NetworkAddress, E>
			where
				E: serde::de::Error,
			{
				let Some((network, address)) = value.split_once('/') else {
					return Err(serde::de::Error::invalid_value(
						serde::de::Unexpected::Str(value),
						&self,
					));
				};
				let Ok(ip_addr) = IpAddr::from_str(address) else {
					return Err(serde::de::Error::invalid_value(
						serde::de::Unexpected::Str(value),
						&self,
					));
				};
				Ok(NetworkAddress {
					network: network.into(),
					address: ip_addr,
				})
			}
		}
		deserializer.deserialize_str(NetworkAddressVisitor)
	}
}

#[derive(Debug, Hash, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Workload {
	pub workload_ips: Vec<IpAddr>,

	#[serde(default, skip_serializing_if = "is_default")]
	pub waypoint: Option<GatewayAddress>,
	#[serde(default, skip_serializing_if = "is_default")]
	pub network_gateway: Option<GatewayAddress>,

	#[serde(default)]
	pub protocol: InboundProtocol,
	#[serde(default)]
	pub network_mode: NetworkMode,

	#[serde(default, skip_serializing_if = "is_default")]
	pub uid: Strng,
	#[serde(default)]
	pub name: Strng,
	pub namespace: Strng,
	#[serde(default, skip_serializing_if = "is_default")]
	pub trust_domain: Strng,
	#[serde(default, skip_serializing_if = "is_default")]
	pub service_account: Strng,
	#[serde(default, skip_serializing_if = "is_default")]
	pub network: Strng,

	#[serde(default, skip_serializing_if = "is_default")]
	pub workload_name: Strng,
	#[serde(default, skip_serializing_if = "is_default")]
	pub workload_type: Strng,
	#[serde(default, skip_serializing_if = "is_default")]
	pub canonical_name: Strng,
	#[serde(default, skip_serializing_if = "is_default")]
	pub canonical_revision: Strng,

	#[serde(default, skip_serializing_if = "is_default")]
	pub hostname: Strng,

	#[serde(default, skip_serializing_if = "is_default")]
	pub node: Strng,

	#[serde(default, skip_serializing_if = "is_default")]
	pub authorization_policies: Vec<Strng>,

	#[serde(default)]
	pub status: HealthStatus,

	#[serde(default)]
	pub cluster_id: Strng,

	#[serde(default, skip_serializing_if = "is_default")]
	pub locality: Locality,

	#[serde(default, skip_serializing_if = "is_default")]
	pub services: Vec<NamespacedHostname>,

	#[serde(default = "default_capacity")]
	pub capacity: u32,
}

fn default_capacity() -> u32 {
	1
}

impl Workload {
	pub fn identity(&self) -> Identity {
		Identity::Spiffe {
			trust_domain: self.trust_domain.clone(),
			namespace: self.namespace.clone(),
			service_account: self.service_account.clone(),
		}
	}
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub enum Identity {
	Spiffe {
		trust_domain: Strng,
		namespace: Strng,
		service_account: Strng,
	},
}

impl EncodeLabelValue for Identity {
	fn encode(&self, writer: &mut LabelValueEncoder) -> Result<(), std::fmt::Error> {
		writer.write_str(&self.to_string())
	}
}

impl serde::Serialize for Identity {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		self.to_string().serialize(serializer)
	}
}

impl FromStr for Identity {
	type Err = anyhow::Error;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		const URI_PREFIX: &str = "spiffe://";
		const SERVICE_ACCOUNT: &str = "sa";
		const NAMESPACE: &str = "ns";
		if !s.starts_with(URI_PREFIX) {
			return Err(anyhow!("invalid spiffe: {s}"));
		}
		let split: Vec<_> = s[URI_PREFIX.len()..].split('/').collect();
		if split.len() != 5 {
			return Err(anyhow!("invalid spiffe: {s}"));
		}
		if split[1] != NAMESPACE || split[3] != SERVICE_ACCOUNT {
			return Err(anyhow!("invalid spiffe: {s}"));
		}
		Ok(Identity::Spiffe {
			trust_domain: split[0].into(),
			namespace: split[2].into(),
			service_account: split[4].into(),
		})
	}
}

impl Display for Identity {
	fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
		match self {
			Identity::Spiffe {
				trust_domain,
				namespace,
				service_account,
			} => {
				write!(
					f,
					"spiffe://{trust_domain}/ns/{namespace}/sa/{service_account}"
				)
			},
		}
	}
}

impl Identity {
	pub fn from_parts(td: Strng, ns: Strng, sa: Strng) -> Identity {
		Identity::Spiffe {
			trust_domain: td,
			namespace: ns,
			service_account: sa,
		}
	}

	pub fn to_strng(self: &Identity) -> Strng {
		match self {
			Identity::Spiffe {
				trust_domain,
				namespace,
				service_account,
			} => {
				strng::format!("spiffe://{trust_domain}/ns/{namespace}/sa/{service_account}")
			},
		}
	}

	pub fn trust_domain(&self) -> Strng {
		match self {
			Identity::Spiffe { trust_domain, .. } => trust_domain.clone(),
		}
	}
}

fn is_default<T: Default + PartialEq>(t: &T) -> bool {
	*t == Default::default()
}

// The protocol that the sender should use to send data. Can be different from ServerProtocol when there is a
// agentgateway in the middle (e.g. e/w gateway with double hbone).
#[derive(
	Default,
	Debug,
	Hash,
	Eq,
	PartialEq,
	Ord,
	PartialOrd,
	Clone,
	Copy,
	serde::Serialize,
	serde::Deserialize,
)]
pub enum OutboundProtocol {
	#[default]
	TCP,
	HBONE,
	DOUBLEHBONE,
}

#[derive(
	Default, Debug, Hash, Eq, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize,
)]
pub enum NetworkMode {
	#[default]
	Standard,
	HostNetwork,
}

#[derive(
	Default, Debug, Hash, Eq, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize,
)]
pub enum HealthStatus {
	#[default]
	Healthy,
	Unhealthy,
}

#[derive(Default, Debug, Hash, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub struct Locality {
	pub region: Strng,
	pub zone: Strng,
	pub subzone: Strng,
}

#[derive(Debug, Hash, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayAddress {
	pub destination: gatewayaddress::Destination,
	pub hbone_mtls_port: u16,
}

pub mod gatewayaddress {
	use super::{NamespacedHostname, NetworkAddress};
	#[derive(Debug, Hash, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
	#[serde(untagged)]
	pub enum Destination {
		Address(NetworkAddress),
		Hostname(NamespacedHostname),
	}
}

// The protocol that the final workload expects
#[derive(
	Default,
	Debug,
	Hash,
	Eq,
	PartialEq,
	Ord,
	PartialOrd,
	Clone,
	Copy,
	serde::Serialize,
	serde::Deserialize,
)]
pub enum InboundProtocol {
	#[default]
	TCP,
	HBONE,
	LegacyIstioMtls,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Service {
	pub name: Strng,
	pub namespace: Strng,
	pub hostname: Strng,
	pub vips: Vec<NetworkAddress>,
	pub ports: HashMap<u16, u16>,
	#[serde(default)]
	pub app_protocols: HashMap<u16, AppProtocol>,

	/// Maps endpoint UIDs to service [Endpoint]s.
	#[serde(default, skip_deserializing)]
	pub endpoints: loadbalancer::EndpointSet<Endpoint>,
	#[serde(default)]
	pub subject_alt_names: Vec<Strng>,

	#[serde(default, skip_serializing_if = "is_default")]
	pub waypoint: Option<GatewayAddress>,

	#[serde(default, skip_serializing_if = "is_default")]
	pub load_balancer: Option<LoadBalancer>,

	#[serde(default, skip_serializing_if = "is_default")]
	pub ip_families: Option<IpFamily>,
}

impl Service {
	pub fn port_is_http2(&self, port: u16) -> bool {
		matches!(
			self.app_protocols.get(&port),
			Some(AppProtocol::Http2 | AppProtocol::Grpc)
		)
	}
	pub fn port_is_http1(&self, port: u16) -> bool {
		matches!(self.app_protocols.get(&port), Some(AppProtocol::Http11))
	}
	pub fn namespaced_hostname(&self) -> NamespacedHostname {
		NamespacedHostname {
			namespace: self.namespace.clone(),
			hostname: self.hostname.clone(),
		}
	}
	pub fn should_include_endpoint(&self, ep_health: HealthStatus) -> bool {
		ep_health == HealthStatus::Healthy
			|| self
				.load_balancer
				.as_ref()
				.map(|lb| lb.health_policy == LoadBalancerHealthPolicy::AllowAll)
				.unwrap_or(false)
	}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum AppProtocol {
	Http11,
	Http2,
	Grpc,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum IpFamily {
	Dual,
	IPv4,
	IPv6,
}

impl IpFamily {
	/// accepts_ip returns true if the provided IP is supposed by the IP family
	pub fn accepts_ip(&self, ip: IpAddr) -> bool {
		match self {
			IpFamily::Dual => true,
			IpFamily::IPv4 => ip.is_ipv4(),
			IpFamily::IPv6 => ip.is_ipv6(),
		}
	}
}

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoadBalancer {
	pub routing_preferences: Vec<LoadBalancerScopes>,
	pub mode: LoadBalancerMode,
	pub health_policy: LoadBalancerHealthPolicy,
}

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub enum LoadBalancerScopes {
	Region,
	Zone,
	Subzone,
	Node,
	Cluster,
	Network,
}

#[derive(Default, Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub enum LoadBalancerHealthPolicy {
	#[default]
	OnlyHealthy,
	AllowAll,
}
#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
pub enum LoadBalancerMode {
	// Do not consider LoadBalancerScopes when picking endpoints
	Standard,
	// Only select endpoints matching all LoadBalancerScopes when picking endpoints; otherwise, fail.
	Strict,
	// Prefer select endpoints matching all LoadBalancerScopes when picking endpoints but allow mismatches
	Failover,
}

#[derive(Debug, Eq, PartialEq, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Endpoint {
	/// The workload UID for this endpoint.
	pub workload_uid: Strng,

	/// The port mapping.
	pub port: HashMap<u16, u16>,

	/// Health status for the endpoint
	pub status: HealthStatus,
}

impl From<workload::TunnelProtocol> for InboundProtocol {
	fn from(value: workload::TunnelProtocol) -> Self {
		match value {
			workload::TunnelProtocol::Hbone => InboundProtocol::HBONE,
			workload::TunnelProtocol::LegacyIstioMtls => InboundProtocol::LegacyIstioMtls,
			workload::TunnelProtocol::None => InboundProtocol::TCP,
		}
	}
}

impl From<workload::NetworkMode> for NetworkMode {
	fn from(value: workload::NetworkMode) -> Self {
		match value {
			workload::NetworkMode::Standard => NetworkMode::Standard,
			workload::NetworkMode::HostNetwork => NetworkMode::HostNetwork,
		}
	}
}
impl From<workload::Locality> for Locality {
	fn from(value: workload::Locality) -> Self {
		Locality {
			region: value.region.into(),
			zone: value.zone.into(),
			subzone: value.subzone.into(),
		}
	}
}

impl From<workload::WorkloadStatus> for HealthStatus {
	fn from(value: workload::WorkloadStatus) -> Self {
		match value {
			workload::WorkloadStatus::Healthy => HealthStatus::Healthy,
			workload::WorkloadStatus::Unhealthy => HealthStatus::Unhealthy,
		}
	}
}
impl From<&PortList> for HashMap<u16, u16> {
	fn from(value: &PortList) -> Self {
		value
			.ports
			.iter()
			.map(|p| (p.service_port as u16, p.target_port as u16))
			.collect()
	}
}

impl From<HashMap<u16, u16>> for PortList {
	fn from(value: HashMap<u16, u16>) -> Self {
		PortList {
			ports: value
				.iter()
				.map(|(k, v)| Port {
					service_port: *k as u32,
					app_protocol: 0,
					target_port: *v as u32,
				})
				.collect(),
		}
	}
}

impl TryFrom<&XdsGatewayAddress> for GatewayAddress {
	type Error = ProtoError;

	fn try_from(value: &workload::GatewayAddress) -> Result<Self, Self::Error> {
		let gw_addr: GatewayAddress = match &value.destination {
			Some(a) => match a {
				workload::gateway_address::Destination::Address(addr) => GatewayAddress {
					destination: gatewayaddress::Destination::Address(network_addr(
						strng::new(&addr.network),
						byte_to_ip(&Bytes::copy_from_slice(&addr.address))?,
					)),
					hbone_mtls_port: value.hbone_mtls_port as u16,
				},
				workload::gateway_address::Destination::Hostname(hn) => GatewayAddress {
					destination: gatewayaddress::Destination::Hostname(NamespacedHostname {
						namespace: Strng::from(&hn.namespace),
						hostname: Strng::from(&hn.hostname),
					}),
					hbone_mtls_port: value.hbone_mtls_port as u16,
				},
			},
			None => return Err(ProtoError::MissingGatewayAddress),
		};
		Ok(gw_addr)
	}
}

impl TryFrom<XdsWorkload> for Workload {
	type Error = ProtoError;
	fn try_from(resource: XdsWorkload) -> Result<Self, Self::Error> {
		let (w, _): (Workload, HashMap<String, PortList>) = resource.try_into()?;
		Ok(w)
	}
}

impl TryFrom<XdsWorkload> for (Workload, HashMap<String, PortList>) {
	type Error = ProtoError;
	fn try_from(resource: XdsWorkload) -> Result<Self, Self::Error> {
		let wp = match &resource.waypoint {
			Some(w) => Some(GatewayAddress::try_from(w)?),
			None => None,
		};

		let network_gw = match &resource.network_gateway {
			Some(w) => Some(GatewayAddress::try_from(w)?),
			None => None,
		};

		let addresses = resource
			.addresses
			.iter()
			.map(byte_to_ip)
			.collect::<Result<Vec<_>, _>>()?;

		let workload_type = resource.workload_type().as_str_name().to_lowercase();
		let services: Vec<NamespacedHostname> = resource
			.services
			.keys()
			.map(|namespaced_host| match namespaced_host.split_once('/') {
				Some((namespace, hostname)) => Ok(NamespacedHostname {
					namespace: namespace.into(),
					hostname: hostname.into(),
				}),
				None => Err(ProtoError::NamespacedHostnameParse(namespaced_host.clone())),
			})
			.collect::<Result<_, _>>()?;
		let wl = Workload {
			workload_ips: addresses,
			waypoint: wp,
			network_gateway: network_gw,

			protocol: InboundProtocol::from(workload::TunnelProtocol::try_from(
				resource.tunnel_protocol,
			)?),
			network_mode: NetworkMode::from(workload::NetworkMode::try_from(resource.network_mode)?),

			uid: resource.uid.into(),
			name: resource.name.into(),
			namespace: resource.namespace.into(),
			trust_domain: {
				let result = resource.trust_domain;
				if result.is_empty() {
					"cluster.local".into()
				} else {
					result.into()
				}
			},
			service_account: {
				let result = resource.service_account;
				if result.is_empty() {
					"default".into()
				} else {
					result.into()
				}
			},
			node: resource.node.into(),
			hostname: resource.hostname.into(),
			network: resource.network.into(),
			workload_name: resource.workload_name.into(),
			workload_type: workload_type.into(),
			canonical_name: resource.canonical_name.into(),
			canonical_revision: resource.canonical_revision.into(),

			status: HealthStatus::from(workload::WorkloadStatus::try_from(resource.status)?),

			authorization_policies: resource
				.authorization_policies
				.iter()
				.map(strng::new)
				.collect(),

			locality: resource.locality.map(Locality::from).unwrap_or_default(),

			cluster_id: {
				let result = resource.cluster_id;
				if result.is_empty() {
					"Kubernetes".into()
				} else {
					result.into()
				}
			},

			capacity: resource.capacity.unwrap_or(1),
			services,
		};
		// Return back part we did not use (service) so it can be consumed without cloning
		Ok((wl, resource.services))
	}
}

pub fn byte_to_ip(b: &Bytes) -> Result<IpAddr, ProtoError> {
	match b.len() {
		4 => {
			let v: [u8; 4] = b.deref().try_into().expect("size already proven");
			Ok(IpAddr::from(v))
		},
		16 => {
			let v: [u8; 16] = b.deref().try_into().expect("size already proven");
			Ok(IpAddr::from(v))
		},
		n => Err(ProtoError::ByteAddressParse(n)),
	}
}

pub fn network_addr(network: Strng, vip: IpAddr) -> NetworkAddress {
	NetworkAddress {
		network,
		address: vip,
	}
}

impl TryFrom<&XdsService> for Service {
	type Error = ProtoError;

	fn try_from(s: &XdsService) -> Result<Self, Self::Error> {
		let mut nw_addrs = Vec::new();
		for addr in &s.addresses {
			let network_address = network_addr(
				strng::new(&addr.network),
				byte_to_ip(&Bytes::copy_from_slice(&addr.address))?,
			);
			nw_addrs.push(network_address);
		}
		let waypoint = match &s.waypoint {
			Some(w) => Some(GatewayAddress::try_from(w)?),
			None => None,
		};
		let lb = if let Some(lb) = &s.load_balancing {
			Some(LoadBalancer {
				routing_preferences: lb
					.routing_preference
					.iter()
					.map(|r| {
						workload::load_balancing::Scope::try_from(*r)
							.map_err(ProtoError::EnumError)
							.and_then(|r| r.try_into())
					})
					.collect::<Result<Vec<LoadBalancerScopes>, ProtoError>>()?,
				mode: workload::load_balancing::Mode::try_from(lb.mode)?.into(),
				health_policy: workload::load_balancing::HealthPolicy::try_from(lb.health_policy)?.into(),
			})
		} else {
			None
		};
		let app_protocols = s
			.ports
			.iter()
			.map(|p| {
				let ap = workload::AppProtocol::try_from(p.app_protocol)?;
				let ap = <Option<AppProtocol>>::from(ap);
				Ok(ap.map(|ap| (p.service_port as u16, ap)))
			})
			.filter_map(|v| match v {
				Ok(None) => None,
				Ok(Some(ap)) => Some(Ok(ap)),
				Err(e) => Some(Err(e)),
			})
			.collect::<Result<HashMap<_, _>, ProtoError>>()?;
		let ip_families = workload::IpFamilies::try_from(s.ip_families)?.into();
		let svc = Service {
			name: Strng::from(&s.name),
			namespace: Strng::from(&s.namespace),
			hostname: Strng::from(&s.hostname),
			vips: nw_addrs,
			ports: (&PortList {
				ports: s.ports.clone(),
			})
				.into(),
			app_protocols,
			endpoints: Default::default(), // Will be populated once inserted into the store.
			subject_alt_names: s.subject_alt_names.iter().map(strng::new).collect(),
			waypoint,
			load_balancer: lb,
			ip_families,
		};
		Ok(svc)
	}
}

impl From<workload::IpFamilies> for Option<IpFamily> {
	fn from(value: workload::IpFamilies) -> Self {
		match value {
			workload::IpFamilies::Automatic => None,
			workload::IpFamilies::Ipv4Only => Some(IpFamily::IPv4),
			workload::IpFamilies::Ipv6Only => Some(IpFamily::IPv6),
			workload::IpFamilies::Dual => Some(IpFamily::Dual),
		}
	}
}

impl From<workload::AppProtocol> for Option<AppProtocol> {
	fn from(value: workload::AppProtocol) -> Self {
		match value {
			workload::AppProtocol::Unknown => None,
			workload::AppProtocol::Http11 => Some(AppProtocol::Http11),
			workload::AppProtocol::Http2 => Some(AppProtocol::Http2),
			workload::AppProtocol::Grpc => Some(AppProtocol::Grpc),
		}
	}
}

impl From<workload::load_balancing::Mode> for LoadBalancerMode {
	fn from(value: workload::load_balancing::Mode) -> Self {
		match value {
			workload::load_balancing::Mode::Strict => LoadBalancerMode::Strict,
			workload::load_balancing::Mode::Failover => LoadBalancerMode::Failover,
			workload::load_balancing::Mode::UnspecifiedMode => LoadBalancerMode::Standard,
		}
	}
}

impl From<workload::load_balancing::HealthPolicy> for LoadBalancerHealthPolicy {
	fn from(value: workload::load_balancing::HealthPolicy) -> Self {
		match value {
			workload::load_balancing::HealthPolicy::OnlyHealthy => LoadBalancerHealthPolicy::OnlyHealthy,
			workload::load_balancing::HealthPolicy::AllowAll => LoadBalancerHealthPolicy::AllowAll,
		}
	}
}

impl TryFrom<XdsScope> for LoadBalancerScopes {
	type Error = ProtoError;
	fn try_from(value: XdsScope) -> Result<Self, Self::Error> {
		match value {
			XdsScope::Region => Ok(LoadBalancerScopes::Region),
			XdsScope::Zone => Ok(LoadBalancerScopes::Zone),
			XdsScope::Subzone => Ok(LoadBalancerScopes::Subzone),
			XdsScope::Node => Ok(LoadBalancerScopes::Node),
			XdsScope::Cluster => Ok(LoadBalancerScopes::Cluster),
			XdsScope::Network => Ok(LoadBalancerScopes::Network),
			_ => Err(ProtoError::EnumParse("invalid target".to_string())),
		}
	}
}
