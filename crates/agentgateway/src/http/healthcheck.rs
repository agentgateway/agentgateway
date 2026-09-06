//! Active health checks for LLM provider endpoints.
//!
//! Passive health only learns that a provider is down by failing a real request. An active check
//! probes each provider on a timer, keeps a failing provider evicted for as long as it keeps
//! failing, and lets it back in once it answers again. A replica that is down for days therefore
//! takes none of the traffic, and rejoins without operator action when it recovers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::http::health::ActiveHealthCheck;
use crate::llm::{NamedAIProvider, RouteType};
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::{
	Backend, BackendKey, BackendTargetRef, BackendWithPolicies, ResourceName, SimpleBackend, Target,
};
use crate::*;

const HEALTH_CHECK_USER_AGENT: &str = "agentgateway-health-check";

/// One provider that has an active check configured.
struct ProbeTarget {
	backend: Arc<BackendWithPolicies>,
	backend_key: BackendKey,
	provider: Arc<NamedAIProvider>,
	policies: BackendPolicies,
	check: ActiveHealthCheck,
	restore_health: Option<f64>,
}

#[derive(Debug, Default)]
struct ProbeState {
	next_at: Option<Instant>,
	healthy_streak: u32,
	unhealthy_streak: u32,
	/// Whether this checker holds the provider evicted.
	evicted: bool,
}

/// Probes LLM providers and feeds the results into the load balancer's health state.
pub struct HealthChecker {
	inputs: Arc<ProxyInputs>,
	state: HashMap<(BackendKey, Strng), ProbeState>,
}

impl HealthChecker {
	pub fn new(inputs: Arc<ProxyInputs>) -> Self {
		Self {
			inputs,
			state: HashMap::new(),
		}
	}

	/// Run until the process exits, probing every provider whose health policy has an active check.
	pub async fn run(mut self) {
		let mut tick = tokio::time::interval(Duration::from_secs(1));
		tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
		loop {
			tick.tick().await;
			self.run_once().await;
		}
	}

	/// Probe every provider that is due. Returns how many probes were sent.
	pub async fn run_once(&mut self) -> usize {
		let targets = self.collect_targets();
		let now = Instant::now();
		let mut seen = std::collections::HashSet::new();
		let mut due = Vec::new();
		for target in targets {
			let key = (target.backend_key.clone(), target.provider.name.clone());
			seen.insert(key.clone());
			let state = self.state.entry(key).or_default();
			if state.next_at.is_some_and(|t| t > now) {
				continue;
			}
			state.next_at = Some(now + target.check.interval);
			due.push(target);
		}
		self.state.retain(|k, _| seen.contains(k));

		let probes = due.iter().map(|target| self.probe(target));
		let results = futures_util::future::join_all(probes).await;
		let count = results.len();
		for (target, (healthy, latency)) in due.iter().zip(results) {
			self.record(target, healthy, latency);
		}
		count
	}

	fn collect_targets(&self) -> Vec<ProbeTarget> {
		let binds = self.inputs.stores.read_binds();
		let mut targets = Vec::new();
		for backend in binds.backends() {
			let Backend::AI(name, ai) = &backend.backend else {
				continue;
			};
			let backend_key = backend.backend.name();
			let backend_policies = binds.backend_policies(
				BackendTargetRef::Backend {
					name: name.name.as_ref(),
					namespace: name.namespace.as_ref(),
					section: None,
				},
				&[backend.inline_policies.as_slice()],
				None,
			);
			let mut providers: Vec<Arc<NamedAIProvider>> = Vec::new();
			ai.providers.find_endpoint(|ep, _| {
				providers.push(ep.clone());
				None::<()>
			});
			for provider in providers {
				let sub = binds.sub_backend_policies(
					BackendTargetRef::Backend {
						name: name.name.as_ref(),
						namespace: name.namespace.as_ref(),
						section: Some(provider.name.as_ref()),
					},
					Some(&provider.inline_policies),
				);
				let policies = backend_policies.clone().merge(sub);
				let Some(health) = policies.health.as_ref() else {
					continue;
				};
				let Some(check) = health.active.clone() else {
					continue;
				};
				targets.push(ProbeTarget {
					backend: backend.clone(),
					backend_key: backend_key.clone(),
					provider,
					restore_health: health.eviction.as_ref().and_then(|e| e.restore_health),
					policies,
					check,
				});
			}
		}
		targets
	}

	/// Resolve where the provider's requests go, the same way the proxy does.
	fn resolve(&self, target: &ProbeTarget) -> Option<(SimpleBackend, BackendPolicies)> {
		let provider = &target.provider;
		let mut policies = target.policies.clone();
		let backend = if let Some(reference) = &provider.provider_backend {
			let resolved =
				crate::proxy::resolve_simple_backend_with_policies(reference, self.inputs.as_ref()).ok()?;
			let extra = self.inputs.stores.read_binds().backend_policies(
				resolved.backend.target(),
				&[&resolved.inline_policies],
				None,
			);
			policies = policies.merge(extra);
			resolved.backend
		} else {
			let host = match &provider.host_override {
				Some(host) => host.clone(),
				None => {
					if let Some(defaults) = provider.provider.default_connector_policies() {
						policies = defaults.merge(policies);
					}
					provider
						.provider
						.default_connector_target(RouteType::Completions)?
				},
			};
			SimpleBackend::Opaque(
				ResourceName::new(strng::format!("{host}"), strng::EMPTY),
				host,
			)
		};
		// The probe is a plain HTTP request: no LLM processing, and no passive health accounting.
		policies.llm_provider = None;
		policies.llm = None;
		policies.health = None;
		policies.inference_routing = None;
		Some((backend, policies))
	}

	async fn probe(&self, target: &ProbeTarget) -> (bool, Duration) {
		let start = Instant::now();
		let Some((backend, policies)) = self.resolve(target) else {
			warn!(
				backend = %target.backend_key,
				provider = %target.provider.name,
				"active health check: cannot resolve provider target"
			);
			return (false, start.elapsed());
		};
		let healthy = self
			.send_probe(target, backend, policies)
			.await
			.unwrap_or_else(|why| {
				debug!(
					backend = %target.backend_key,
					provider = %target.provider.name,
					"active health check {why}"
				);
				false
			});
		(healthy, start.elapsed())
	}

	/// Send one probe. The error says why it reached no verdict.
	async fn send_probe(
		&self,
		target: &ProbeTarget,
		backend: SimpleBackend,
		policies: BackendPolicies,
	) -> Result<bool, String> {
		// The client sets the scheme from the backend's transport and uses the authority as the
		// Host header, so the probe needs the same absolute form a proxied request has.
		let authority = probe_authority(&backend).ok_or("skipped: backend type cannot be probed")?;
		let req = ::http::Request::builder()
			.method(::http::Method::GET)
			.uri(format!("http://{authority}{}", target.check.path))
			.header(::http::header::USER_AGENT, HEALTH_CHECK_USER_AGENT)
			.body(crate::http::Body::empty())
			.map_err(|e| format!("skipped: {e}"))?;
		let client = PolicyClient::new(self.inputs.clone());
		let call = client.call_with_explicit_policies_untraced(req, &backend, policies);
		match tokio::time::timeout(target.check.timeout, call).await {
			Ok(Ok(resp)) => Ok(target.check.accepts(resp.status())),
			Ok(Err(e)) => Err(format!("failed: {e}")),
			Err(_) => Err("timed out".to_string()),
		}
	}

	fn record(&mut self, target: &ProbeTarget, healthy: bool, latency: Duration) {
		let Backend::AI(_, ai) = &target.backend.backend else {
			return;
		};
		let provider = target.provider.name.clone();
		let Some(info) = ai
			.providers
			.find_endpoint(|ep, info| (ep.name == provider).then(|| info.clone()))
		else {
			return;
		};
		// Feed the sample into the same EWMA and failure counters real traffic uses.
		ai.providers
			.start_request(provider.clone(), &info)
			.finish_request(healthy, latency, None, None);

		let key = (target.backend_key.clone(), provider.clone());
		let Some(state) = self.state.get_mut(&key) else {
			return;
		};
		if healthy {
			state.unhealthy_streak = 0;
			state.healthy_streak = state.healthy_streak.saturating_add(1);
			// Only evictions made by the checker are lifted here. An eviction made from real
			// traffic keeps its own duration, so both signals have to agree before the provider
			// takes requests again.
			if state.evicted && state.healthy_streak >= target.check.healthy_threshold {
				info!(
					backend = %target.backend_key,
					provider = %provider,
					"active health check: provider is healthy again"
				);
				if info.is_evicted() {
					ai.providers
						.evict_until(provider, Instant::now(), target.restore_health);
				}
				state.evicted = false;
			}
		} else {
			state.healthy_streak = 0;
			state.unhealthy_streak = state.unhealthy_streak.saturating_add(1);
			if state.unhealthy_streak >= target.check.unhealthy_threshold {
				if !state.evicted {
					warn!(
						backend = %target.backend_key,
						provider = %provider,
						"active health check: evicting provider after {} failed probes",
						state.unhealthy_streak
					);
				}
				// Keep the provider out until the next probe has had a chance to run.
				let until = Instant::now() + target.check.interval * 2 + target.check.timeout;
				ai.providers.evict_until(provider, until, None);
				state.evicted = true;
			}
		}
	}
}

/// The `host[:port]` a probe request is addressed to.
fn probe_authority(backend: &SimpleBackend) -> Option<String> {
	match backend {
		SimpleBackend::Service(svc, port) => Some(format!("{}:{port}", svc.hostname)),
		SimpleBackend::Opaque(_, Target::UnixSocket(_)) => Some("localhost".to_string()),
		SimpleBackend::Opaque(_, target) => Some(target.to_string()),
		SimpleBackend::Aws(_, _) | SimpleBackend::Invalid => None,
	}
}

#[cfg(test)]
#[path = "healthcheck_tests.rs"]
mod tests;
