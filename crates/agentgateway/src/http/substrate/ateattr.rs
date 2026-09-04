//! Telemetry conventions owned by Agent Substrate, not by agentgateway.
//!
//! Mirrors `internal/ateattr/ateattr.go` and `docs/metrics/registry/metrics.yaml` in
//! agent-substrate/substrate, which are the source of truth. Strings agentgateway chose for itself
//! do not belong here; see `telemetry/semconv.rs` for the OpenTelemetry conventions.

use std::fmt::{Display, Formatter};

/// The atespace-scoped addressable name, mirroring `k8s.pod.name`. Upstream has deliberately no
/// `ate.actor.id`: it is ambiguous once an actor has both a name and a uid.
pub(crate) const ATE_ACTOR_NAME: &str = "ate.actor.name";
pub(crate) const ATE_ACTOR_UID: &str = "ate.actor.uid";
pub(crate) const ATE_ATESPACE: &str = "ate.atespace";
pub(crate) const ATE_ROUTER_RESUME: &str = "ate.router.resume";
pub(crate) const ATE_ROUTER_ROUTE_DURATION: &str = "ate.router.route.duration";

/// Whether this request experienced a cold actor activation.
///
/// Ordered weakest to strongest so `max` keeps the strongest disposition a request observed across
/// stale-assignment retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(crate) enum ResumeDisposition {
	#[default]
	None,
	Joined,
	Triggered,
}

impl ResumeDisposition {
	pub(crate) fn as_str(self) -> &'static str {
		match self {
			Self::None => "none",
			Self::Joined => "joined",
			Self::Triggered => "triggered",
		}
	}

	pub(crate) fn for_resumed(resumed: bool, joined: bool) -> Self {
		match (resumed, joined) {
			(false, _) => Self::None,
			(true, false) => Self::Triggered,
			(true, true) => Self::Joined,
		}
	}
}

impl Display for ResumeDisposition {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.as_str())
	}
}

#[cfg(test)]
mod tests {
	use super::ResumeDisposition;

	#[test]
	fn disposition_values_match_upstream_ateattr() {
		assert_eq!(ResumeDisposition::None.as_str(), "none");
		assert_eq!(ResumeDisposition::Triggered.as_str(), "triggered");
		assert_eq!(ResumeDisposition::Joined.as_str(), "joined");
		assert_eq!(super::ATE_ROUTER_RESUME, "ate.router.resume");
		assert_eq!(super::ATE_ATESPACE, "ate.atespace");
		assert_eq!(super::ATE_ACTOR_NAME, "ate.actor.name");
		assert_eq!(super::ATE_ACTOR_UID, "ate.actor.uid");
		assert_eq!(
			super::ATE_ROUTER_ROUTE_DURATION,
			"ate.router.route.duration"
		);
	}

	#[test]
	fn resumed_gates_the_classification() {
		// A leader whose resume was a no-op is not a cold start, and neither is a follower that
		// waited on one.
		assert_eq!(
			ResumeDisposition::for_resumed(false, false),
			ResumeDisposition::None
		);
		assert_eq!(
			ResumeDisposition::for_resumed(false, true),
			ResumeDisposition::None
		);
		assert_eq!(
			ResumeDisposition::for_resumed(true, false),
			ResumeDisposition::Triggered
		);
		assert_eq!(
			ResumeDisposition::for_resumed(true, true),
			ResumeDisposition::Joined
		);
	}

	#[test]
	fn triggered_outranks_joined_outranks_none() {
		assert!(ResumeDisposition::Triggered > ResumeDisposition::Joined);
		assert!(ResumeDisposition::Joined > ResumeDisposition::None);
		assert_eq!(
			ResumeDisposition::default(),
			ResumeDisposition::None,
			"an unresolved request must not read as an activation"
		);
	}
}
