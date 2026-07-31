//! Lazy policy tracing independent of the gateway's concrete trace format.

use std::fmt::{Debug, Formatter};
use std::sync::OnceLock;
use std::time::Instant;

/// Emits a lazily formatted policy event.
///
/// The policy module must define `TRACE_POLICY_KIND`.
#[macro_export]
macro_rules! pol_event {
	($severity:expr, $($arg:tt)+) => {
		$crate::PolicyTrace::event(
			$crate::policy_trace(),
			TRACE_POLICY_KIND,
			$severity,
			&|| format!($($arg)+),
		)
	};
}

/// Emits a lazily formatted policy result.
///
/// The policy module must define `TRACE_POLICY_KIND`.
#[macro_export]
macro_rules! pol_result {
	($severity:expr, $start:expr, $($arg:tt)+) => {
		$crate::PolicyTrace::result_apply(
			$crate::policy_trace(),
			TRACE_POLICY_KIND,
			$severity,
			$start,
			&|| format!($($arg)+),
		)
	};
}

/// Severity attached to a policy trace record.
#[derive(Clone, Copy, Debug)]
pub enum TraceSeverity {
	Info,
	Warn,
	Error,
}

/// Owns a host trace scope until dropped.
#[must_use = "dropping the guard closes the trace scope"]
pub struct TraceScope {
	_guard: Option<Box<dyn Send>>,
}

impl TraceScope {
	/// Wraps a host-owned scope guard.
	pub fn new(scope: impl Send + 'static) -> Self {
		Self {
			_guard: Some(Box::new(scope)),
		}
	}

	/// Creates a scope with no host guard.
	pub fn noop() -> Self {
		Self { _guard: None }
	}
}

impl Debug for TraceScope {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("TraceScope").finish_non_exhaustive()
	}
}

/// Host integration for policy events and timed results.
///
/// Detail closures must remain lazy because formatting can be expensive and is
/// unnecessary when the host does not record the event.
pub trait PolicyTrace: Send + Sync {
	fn timed_start(&self) -> Option<Instant> {
		None
	}

	fn start_scope(&self, _name: &'static str) -> TraceScope {
		TraceScope::noop()
	}

	fn event(&self, kind: &'static str, _severity: TraceSeverity, details: &dyn Fn() -> String) {
		if tracing::enabled!(tracing::Level::DEBUG) {
			tracing::debug!(policy_kind = kind, details = details(), "policy event");
		}
	}

	fn result_apply(
		&self,
		kind: &'static str,
		_severity: TraceSeverity,
		_start: Option<Instant>,
		details: &dyn Fn() -> String,
	) {
		if tracing::enabled!(tracing::Level::DEBUG) {
			tracing::debug!(policy_kind = kind, details = details(), "policy result");
		}
	}
}

/// Default trace implementation, which emits debug tracing records.
pub struct NoopTrace;

impl PolicyTrace for NoopTrace {}

static NOOP_TRACE: NoopTrace = NoopTrace;
static POLICY_TRACE: OnceLock<&'static dyn PolicyTrace> = OnceLock::new();

/// Installs the process-wide policy trace implementation.
///
/// Reinstalling the same singleton is allowed. A different implementation
/// triggers a debug assertion and leaves the original installed.
pub fn install_policy_trace(trace: &'static dyn PolicyTrace) {
	if let Some(installed) = POLICY_TRACE.get() {
		debug_assert!(std::ptr::eq(*installed, trace));
		return;
	}
	let _ = POLICY_TRACE.set(trace);
}

/// Returns the installed policy trace implementation.
pub fn policy_trace() -> &'static dyn PolicyTrace {
	POLICY_TRACE.get().copied().unwrap_or(&NOOP_TRACE)
}
