//! Gateway implementations of the host services used by `agent_policy` policies.

use std::time::Instant;

use agent_policy::{PolicyOutcome, PolicyTrace, TraceSeverity};

use crate::cel::{Executor, Expression};
use crate::http::{Request, Response};
use crate::proxy::dtrace::{self, PolicyResult, Severity};

/// Evaluates policy CEL with the gateway's executor.
pub struct GatewayCel;

impl agent_policy::PolicyCel for GatewayCel {
	fn eval_request<'a>(
		&'a self,
		expression: &'a Expression,
		req: &'a Request,
	) -> Result<cel::Value<'a>, agent_policy::Error> {
		let executor = Executor::new_request(req);
		executor.eval(expression).map(|v| v.as_static())
	}

	fn eval_response<'a>(
		&'a self,
		expression: &'a Expression,
		resp: &'a Response,
	) -> Result<cel::Value<'a>, agent_policy::Error> {
		let executor = Executor::new_response(None, resp);
		executor.eval(expression).map(|v| v.as_static())
	}
}

static GATEWAY_POLICY_TRACE: GatewayPolicyTrace = GatewayPolicyTrace;

/// Routes `agent_policy` trace records to the gateway's debug tracer.
pub fn install_policy_trace() {
	agent_policy::install_policy_trace(&GATEWAY_POLICY_TRACE);
}

struct GatewayPolicyTrace;

impl PolicyTrace for GatewayPolicyTrace {
	fn timed_start(&self) -> Option<Instant> {
		dtrace::timed_start()
	}

	fn event(&self, kind: &'static str, severity: TraceSeverity, details: &dyn Fn() -> String) {
		if tracing::enabled!(tracing::Level::DEBUG) {
			tracing::debug!(policy_kind = kind, "{}", details());
		}
		dtrace::trace(|trace| trace.policy_event(gateway_severity(severity), kind, details()));
	}

	fn result(
		&self,
		kind: &'static str,
		severity: TraceSeverity,
		outcome: PolicyOutcome,
		start: Option<Instant>,
		details: &dyn Fn() -> String,
	) {
		if tracing::enabled!(tracing::Level::DEBUG) {
			tracing::debug!(policy_kind = kind, "{}", details());
		}
		dtrace::trace(|trace| {
			let result = match outcome {
				PolicyOutcome::Apply => PolicyResult::Apply {
					details: details(),
					snapshot: None,
				},
				PolicyOutcome::Skip => PolicyResult::Skip { reason: details() },
			};
			let severity = gateway_severity(severity);
			match start {
				Some(start) => trace.policy_result_timed(start, Instant::now(), severity, kind, result),
				None => trace.policy_result(severity, kind, result),
			}
		});
	}
}

fn gateway_severity(severity: TraceSeverity) -> Severity {
	match severity {
		TraceSeverity::Success => Severity::Success,
		TraceSeverity::Info => Severity::Info,
		TraceSeverity::Warn => Severity::Warn,
		TraceSeverity::Error => Severity::Error,
	}
}
