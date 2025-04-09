use std::error::Error as StdErr;
use std::fmt;
use std::fmt::Formatter;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tracing::Level;
use tracing::{error, instrument, warn};

pub use client::*;
pub use metrics::*;
pub use types::*;

use crate::proto::mcpproxy::dev::common::LocalDataSource;
use crate::proto::mcpproxy::dev::common::local_data_source::Source as XdsSource;
use crate::proto::mcpproxy::dev::rbac::Config as XdsRbac;
use crate::proto::mcpproxy::dev::target::Target as XdsTarget;
use crate::proto::mcpproxy::dev::target::target::Target as XdsTargetSpec;

use self::envoy::service::discovery::v3::DeltaDiscoveryRequest;
use crate::rbac;
use crate::strng::Strng;
use std::collections::HashMap;

use crate::inbound;
use crate::outbound;
use serde::{Deserialize, Serialize};

pub mod client;
pub mod metrics;
mod types;

struct DisplayStatus<'a>(&'a tonic::Status);

impl fmt::Display for DisplayStatus<'_> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let s = &self.0;
		write!(f, "status: {:?}, message: {:?}", s.code(), s.message())?;

		if s.message().to_string().contains("authentication failure") {
			write!(
				f,
				" (hint: check the control plane logs for more information)"
			)?;
		}
		if !s.details().is_empty() {
			if let Ok(st) = std::str::from_utf8(s.details()) {
				write!(f, ", details: {st}")?;
			}
		}
		if let Some(src) = s.source().and_then(|s| s.source()) {
			write!(f, ", source: {src}")?;
			// Error is not public to explicitly match on, so do a fuzzy match
			if format!("{src}").contains("Temporary failure in name resolution") {
				write!(f, " (hint: is the DNS server reachable?)")?;
			}
		}
		Ok(())
	}
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("gRPC error {}", DisplayStatus(.0))]
	GrpcStatus(#[from] tonic::Status),
	#[error("gRPC connection error connecting to {}: {}", .0, DisplayStatus(.1))]
	Connection(String, #[source] tonic::Status),
	/// Attempted to send on a MPSC channel which has been canceled
	#[error(transparent)]
	RequestFailure(#[from] Box<mpsc::error::SendError<DeltaDiscoveryRequest>>),
	#[error("transport error: {0}")]
	Transport(#[from] tonic::transport::Error),
	// #[error("failed to send on demand resource")]
	// OnDemandSend(),
	// #[error("TLS Error: {0}")]
	// TLSError(#[from] tls::Error),
}

/// Updates the [ProxyState] from XDS.
/// All state updates code goes in ProxyStateUpdateMutator, that takes state as a parameter.
/// this guarantees that the state is always locked when it is updated.
#[derive(Clone)]
pub struct ProxyStateUpdateMutator {}

#[derive(Clone)]
pub struct ProxyStateUpdater {
	state: Arc<RwLock<XdsStore>>,
	updater: ProxyStateUpdateMutator,
}

impl ProxyStateUpdater {
	/// Creates a new updater for the given stores. Will prefetch certs when workloads are updated.
	pub fn new(state: Arc<RwLock<XdsStore>>) -> Self {
		Self {
			state,
			updater: ProxyStateUpdateMutator {},
		}
	}
}

impl ProxyStateUpdateMutator {
	#[instrument(
        level = Level::TRACE,
        name="insert_target",
        skip_all,
        fields(name=%target.name),
    )]
	pub fn insert_target(&self, state: &mut XdsStore, target: XdsTarget) -> anyhow::Result<()> {
		let target = outbound::Target::try_from(target)
			.map_err(|e| anyhow::anyhow!("failed to parse target: {e}"))?;
		// TODO: This is a hack
		// TODO: Separate connection/LB from insertion
		state.targets.insert(target);
		Ok(())
	}

	#[instrument(
        level = Level::TRACE,
        name="remove_target",
        skip_all,
        fields(name=%xds_name),
    )]
	pub fn remove_target(&self, state: &mut XdsStore, xds_name: &Strng) {
		state.targets.remove(xds_name);
	}

	#[instrument(
        level = Level::TRACE,
        name="insert_rbac",
        skip_all,
    )]
	pub fn insert_rbac(&self, state: &mut XdsStore, rbac: XdsRbac) -> anyhow::Result<()> {
		let rule_set = rbac::RuleSet::from(&rbac);
		state.policies.insert(rule_set);
		Ok(())
	}

	#[instrument(
        level = Level::TRACE,
        name="remove_rbac",
        skip_all,
        fields(name=%xds_name),
    )]
	pub fn remove_rbac(&self, state: &mut XdsStore, xds_name: &Strng) {
		state.policies.remove(xds_name);
	}
}

#[async_trait::async_trait]
impl Handler<XdsTarget> for ProxyStateUpdater {
	async fn handle(&self, updates: Vec<XdsUpdate<XdsTarget>>) -> Result<(), Vec<RejectedConfig>> {
		let mut state = self.state.write().unwrap();
		let handle = |res: XdsUpdate<XdsTarget>| {
			match res {
				XdsUpdate::Update(w) => self.updater.insert_target(&mut state, w.resource)?,
				XdsUpdate::Remove(name) => self.updater.remove_target(&mut state, &name),
			}
			Ok(())
		};
		handle_single_resource(updates, handle)
	}
}

#[async_trait::async_trait]
impl Handler<XdsRbac> for ProxyStateUpdater {
	async fn handle(&self, updates: Vec<XdsUpdate<XdsRbac>>) -> Result<(), Vec<RejectedConfig>> {
		let mut state = self.state.write().unwrap();
		let handle = |res: XdsUpdate<XdsRbac>| {
			match res {
				XdsUpdate::Update(w) => self.updater.insert_rbac(&mut state, w.resource)?,
				XdsUpdate::Remove(name) => self.updater.remove_rbac(&mut state, &name),
			}
			Ok(())
		};
		handle_single_resource(updates, handle)
	}
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
	#[error("missing fields")]
	MissingFields,
	#[error("invalid schema")]
	InvalidSchema,
}

impl TryFrom<XdsTarget> for outbound::Target {
	type Error = ParseError;
	fn try_from(value: XdsTarget) -> Result<Self, Self::Error> {
		let target = value.target.ok_or(ParseError::MissingFields)?;
		let spec = match target {
			XdsTargetSpec::Sse(sse) => outbound::TargetSpec::Sse {
				host: sse.host.clone(),
				port: sse.port,
				path: sse.path.clone(),
				headers: sse.headers.clone(),
				backend_auth: None,
			},
			XdsTargetSpec::Stdio(stdio) => outbound::TargetSpec::Stdio {
				cmd: stdio.cmd.clone(),
				args: stdio.args.clone(),
				env: stdio.env.clone(),
			},
			XdsTargetSpec::Openapi(openapi) => outbound::TargetSpec::OpenAPI(
				outbound::OpenAPITarget::try_from(openapi).map_err(|_| ParseError::InvalidSchema)?,
			),
		};
		Ok(outbound::Target {
			name: value.name.clone(),
			spec,
		})
	}
}

pub fn resolve_local_data_source(
	local_data_source: &LocalDataSource,
) -> Result<Vec<u8>, ParseError> {
	match local_data_source
		.source
		.as_ref()
		.ok_or(ParseError::MissingFields)?
	{
		XdsSource::FilePath(file_path) => {
			let file = std::fs::read(file_path).map_err(|_| ParseError::MissingFields)?;
			Ok(file)
		},
		XdsSource::Inline(inline) => Ok(inline.clone()),
	}
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TargetStore {
	by_name: HashMap<String, outbound::Target>,
}

impl Default for TargetStore {
	fn default() -> Self {
		Self::new()
	}
}

impl TargetStore {
	pub fn new() -> Self {
		Self {
			by_name: HashMap::new(),
		}
	}

	pub fn remove(&mut self, name: &str) {
		// TODO: Drain connections from target
		self.by_name.remove(name);
	}

	pub fn insert(&mut self, target: outbound::Target) {
		self.by_name.insert(target.name.clone(), target);
	}

	pub fn get(&self, name: &str) -> Option<&outbound::Target> {
		self.by_name.get(name)
	}

	pub fn iter(&self) -> impl Iterator<Item = (String, &outbound::Target)> {
		self
			.by_name
			.iter()
			.map(|(name, target)| (name.clone(), target))
	}

	pub fn clear(&mut self) {
		self.by_name.clear();
	}
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PolicyStore {
	by_name: HashMap<String, rbac::RuleSet>,
}

impl PolicyStore {
	pub fn new() -> Self {
		Self {
			by_name: HashMap::new(),
		}
	}
}

impl Default for PolicyStore {
	fn default() -> Self {
		Self::new()
	}
}

impl PolicyStore {
	pub fn insert(&mut self, policy: rbac::RuleSet) {
		self.by_name.insert(policy.to_key(), policy);
	}

	pub fn remove(&mut self, name: &str) {
		self.by_name.remove(name);
	}

	pub fn validate(&self, resource: &rbac::ResourceType, claims: &rbac::Identity) -> bool {
		self
			.by_name
			.values()
			.any(|policy| policy.validate(resource, claims))
	}

	pub fn clear(&mut self) {
		self.by_name.clear();
	}
}

pub struct XdsStore {
	pub targets: TargetStore,
	pub policies: PolicyStore,
	pub listener: inbound::Listener,
}

impl XdsStore {
	pub fn new(listener: inbound::Listener) -> Self {
		Self {
			targets: TargetStore::new(),
			policies: PolicyStore::new(),
			listener,
		}
	}
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
	pub xds_address: String,
	pub metadata: HashMap<String, String>,
	pub listener: inbound::Listener,
}
