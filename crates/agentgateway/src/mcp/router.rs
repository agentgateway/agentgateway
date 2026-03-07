use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_core::prelude::Strng;
use axum::response::Response;
use openapiv3::OpenAPI;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ProxyInputs;
use crate::http::authorization::RuleSets;
use crate::http::sessionpersistence::{Encoder, MCPSnapshotMember};
use crate::http::*;
use crate::mcp::auth;
use crate::mcp::handler::RelayInputs;
use crate::mcp::session::SessionManager;
use crate::mcp::sse::LegacySSEService;
use crate::mcp::streamablehttp::{StreamableHttpServerConfig, StreamableHttpService};
use crate::mcp::{MCPInfo, McpAuthorizationSet};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::{MustSnapshot, PolicyClient};
use crate::store::{BackendPolicies, Stores};
use crate::telemetry::log::RequestLog;
use crate::types::agent::{
	BackendTargetRef, McpBackend, McpTargetSpec, ResourceName, SimpleBackend, SimpleBackendReference,
	Target,
};

#[derive(Debug, Clone)]
pub struct App {
	state: Stores,
	session: Arc<SessionManager>,
}

impl App {
	pub fn new(state: Stores, session_encoder: Encoder) -> Self {
		let session: Arc<SessionManager> =
			Arc::new(crate::mcp::session::SessionManager::new(session_encoder));
		Self { state, session }
	}

	pub fn should_passthrough(
		&self,
		backend_policies: &BackendPolicies,
		backend: &McpBackend,
		req: &Request,
	) -> Option<SimpleBackendReference> {
		if backend.targets.len() != 1 {
			return None;
		}

		if backend_policies.mcp_authentication.is_some() {
			return None;
		}
		if !req.uri().path().contains("/.well-known/") {
			return None;
		}
		match backend.targets.first().map(|t| &t.spec) {
			Some(McpTargetSpec::Mcp(s)) => Some(s.backend.clone()),
			Some(McpTargetSpec::Sse(s)) => Some(s.backend.clone()),
			_ => None,
		}
	}

	#[allow(clippy::too_many_arguments)]
	pub async fn serve(
		&self,
		pi: Arc<ProxyInputs>,
		backend_group_name: ResourceName,
		backend: McpBackend,
		backend_policies: BackendPolicies,
		mut req: MustSnapshot<'_>,
		mut log: &mut RequestLog,
	) -> Result<Response, ProxyError> {
		let backends = {
			let binds = self.state.read_binds();
			let nt = backend
				.targets
				.iter()
				.map(|t| {
					let be = t
						.spec
						.backend()
						.map(|b| crate::proxy::resolve_simple_backend_with_policies(b, &pi))
						.transpose()?;
					let inline_pols = be.as_ref().map(|pol| pol.inline_policies.as_slice());
					let sub_backend_target = BackendTargetRef::Backend {
						name: backend_group_name.name.as_ref(),
						namespace: backend_group_name.namespace.as_ref(),
						section: Some(t.name.as_ref()),
					};
					let backend_policies = backend_policies
						.clone()
						.merge(binds.sub_backend_policies(sub_backend_target, inline_pols));
					Ok::<_, ProxyError>(Arc::new(McpTarget {
						name: t.name.clone(),
						spec: t.spec.clone(),
						backend: be.map(|b| b.backend),
						backend_policies,
						always_use_prefix: backend.always_use_prefix,
					}))
				})
				.collect::<Result<Vec<_>, _>>()?;

			McpBackendGroup {
				targets: nt,
				stateful: backend.stateful,
				allow_degraded: backend.allow_degraded,
				allow_insecure_multiplex: backend.allow_insecure_multiplex,
			}
		};
		let sm = self.session.clone();
		let client = PolicyClient { inputs: pi.clone() };
		let authorization_policies = backend_policies
			.mcp_authorization
			.unwrap_or_else(|| McpAuthorizationSet::new(RuleSets::from(Vec::new())));
		let authn = backend_policies.mcp_authentication;

		// Store an empty value, we will populate each field async
		let logy = log.mcp_status.clone();
		logy.store(Some(MCPInfo::default()));
		req.extensions_mut().insert(logy);
		let tracer = log.span_writer();
		req.extensions_mut().insert(tracer);

		authorization_policies.register(log.cel.ctx());
		log.cel.ctx().maybe_buffer_request_body(&mut req).await;

		// `response` is not valid here, since we run authz first
		// MCP context is added later. The context is inserted after
		// authentication so it can include verified claims

		if let Some(auth) = authn.as_ref()
			&& let Some(resp) = auth::enforce_authentication(&mut req, auth, &client).await?
		{
			return Ok(resp);
		}

		let mut req = req.take_and_snapshot(Some(&mut log))?;
		// This is an unfortunate clone. The request snapshot is intended to be done at the end of the request,
		// so it strips all of the extensions. However, in MCP land its much trickier for us to do this so
		// we snapshot early... but then we lose the extensions. So we do a clone here.
		let snapshot = log.request_snapshot.clone();
		req.extensions_mut().insert(Arc::new(snapshot));
		if req.uri().path() == "/sse" {
			// Legacy handling
			// Assume this is streamable HTTP otherwise
			let sse = LegacySSEService::new(sm);
			Box::pin(sse.handle(
				req,
				RelayInputs {
					backend: backends.clone(),
					policies: authorization_policies.clone(),
					client: client.clone(),
				},
			))
			.await
		} else {
			let streamable = StreamableHttpService::new(
				sm,
				StreamableHttpServerConfig {
					stateful_mode: backend.stateful,
				},
			);
			Box::pin(streamable.handle(
				req,
				RelayInputs {
					backend: backends.clone(),
					policies: authorization_policies.clone(),
					client: client.clone(),
				},
			))
			.await
		}
	}
}
#[derive(Debug, Clone)]
pub struct McpBackendGroup {
	pub targets: Vec<Arc<McpTarget>>,
	pub stateful: bool,
	pub allow_degraded: bool,
	pub allow_insecure_multiplex: bool,
}

impl McpBackendGroup {
	pub fn matches_snapshot_members(
		&self,
		members: &[MCPSnapshotMember],
	) -> Result<bool, serde_json::Error> {
		// Resume is strict-by-identity: an existing session may only bind back to
		// the same named targets with the same stable target definitions. Newly
		// healthy targets do not late-join, and same-name drift is non-resumable.
		let targets_by_name: HashMap<&str, &Arc<McpTarget>> = self
			.targets
			.iter()
			.map(|target| (target.name.as_str(), target))
			.collect();
		let mut seen = HashSet::new();
		for member in members {
			if !seen.insert(member.target.as_str()) {
				tracing::warn!(target = %member.target, "duplicate target found in mcp session snapshot");
				return Ok(false);
			}
			let Some(target) = targets_by_name.get(member.target.as_str()) else {
				tracing::warn!(target = %member.target, "mcp session snapshot target no longer exists");
				return Ok(false);
			};
			let current_fingerprint = target.snapshot_fingerprint()?;
			if current_fingerprint != member.target_fingerprint {
				tracing::warn!(
					target = %member.target,
					expected_fingerprint = %member.target_fingerprint,
					current_fingerprint = %current_fingerprint,
					"mcp session snapshot target fingerprint changed"
				);
				return Ok(false);
			}
		}
		Ok(true)
	}

	pub fn snapshot_subset<'a>(
		&self,
		target_names: impl IntoIterator<Item = &'a str>,
		allow_degraded: bool,
	) -> Option<Self> {
		let targets_by_name: HashMap<&str, Arc<McpTarget>> = self
			.targets
			.iter()
			.map(|target| (target.name.as_str(), target.clone()))
			.collect();
		let mut seen = HashSet::new();
		let mut targets = Vec::new();
		for name in target_names {
			if !seen.insert(name) {
				return None;
			}
			let target = targets_by_name.get(name)?.clone();
			targets.push(target);
		}
		Some(Self {
			targets,
			stateful: self.stateful,
			allow_degraded,
			allow_insecure_multiplex: self.allow_insecure_multiplex,
		})
	}
}

#[derive(Debug)]
pub struct McpTarget {
	pub name: Strng,
	pub spec: crate::types::agent::McpTargetSpec,
	pub backend_policies: BackendPolicies,
	pub backend: Option<SimpleBackend>,
	pub always_use_prefix: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct McpTargetFingerprint<'a> {
	name: &'a str,
	#[serde(flatten)]
	spec: McpTargetFingerprintSpec<'a>,
}

#[derive(Serialize)]
#[serde(tag = "transport", rename_all = "camelCase")]
enum McpTargetFingerprintSpec<'a> {
	Sse {
		backend: StableBackendFingerprint<'a>,
		path: &'a str,
	},
	Mcp {
		backend: StableBackendFingerprint<'a>,
		path: &'a str,
	},
	Stdio {
		cmd: &'a str,
		args: &'a [String],
		env: &'a HashMap<String, String>,
	},
	Openapi {
		backend: StableBackendFingerprint<'a>,
		schema: &'a OpenAPI,
	},
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StableBackendFingerprint<'a> {
	Service {
		hostname: &'a str,
		namespace: &'a str,
		port: u16,
	},
	Host {
		target: &'a Target,
	},
	Invalid,
	Reference {
		backend: &'a SimpleBackendReference,
	},
}

impl McpTarget {
	fn stable_backend_fingerprint<'a>(
		&'a self,
		fallback: &'a SimpleBackendReference,
	) -> StableBackendFingerprint<'a> {
		match self.backend.as_ref() {
			Some(SimpleBackend::Service(service, port)) => StableBackendFingerprint::Service {
				hostname: service.hostname.as_ref(),
				namespace: service.namespace.as_ref(),
				port: *port,
			},
			Some(SimpleBackend::Opaque(_, target)) => StableBackendFingerprint::Host { target },
			Some(SimpleBackend::Invalid) => StableBackendFingerprint::Invalid,
			None => StableBackendFingerprint::Reference { backend: fallback },
		}
	}

	pub(crate) fn snapshot_fingerprint(&self) -> Result<String, serde_json::Error> {
		// Fingerprint the logical target definition, not ephemeral runtime state
		// like a currently resolved pod IP. That keeps restart/resume stable while
		// still rejecting same-name config drift.
		let spec = match &self.spec {
			McpTargetSpec::Sse(sse) => McpTargetFingerprintSpec::Sse {
				backend: self.stable_backend_fingerprint(&sse.backend),
				path: &sse.path,
			},
			McpTargetSpec::Mcp(mcp) => McpTargetFingerprintSpec::Mcp {
				backend: self.stable_backend_fingerprint(&mcp.backend),
				path: &mcp.path,
			},
			McpTargetSpec::Stdio { cmd, args, env } => McpTargetFingerprintSpec::Stdio { cmd, args, env },
			McpTargetSpec::OpenAPI(openapi) => McpTargetFingerprintSpec::Openapi {
				backend: self.stable_backend_fingerprint(&openapi.backend),
				schema: openapi.schema.as_ref(),
			},
		};
		let fingerprint = McpTargetFingerprint {
			name: self.name.as_str(),
			spec,
		};
		let canonical = canonicalize_json_value(serde_json::to_value(fingerprint)?);
		let encoded = serde_json::to_vec(&canonical)?;
		let mut hasher = Sha256::new();
		hasher.update(encoded);
		Ok(hex::encode(hasher.finalize()))
	}
}

fn canonicalize_json_value(value: Value) -> Value {
	match value {
		Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_json_value).collect()),
		Value::Object(map) => {
			let mut keys = map.keys().cloned().collect::<Vec<_>>();
			keys.sort_unstable();
			let mut canonical = serde_json::Map::with_capacity(keys.len());
			for key in keys {
				let value = map
					.get(&key)
					.expect("sorted keys must exist in source map")
					.clone();
				canonical.insert(key, canonicalize_json_value(value));
			}
			Value::Object(canonical)
		},
		other => other,
	}
}
