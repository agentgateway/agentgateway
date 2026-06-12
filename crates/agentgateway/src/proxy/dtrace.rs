use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};

pub use Severity::*;
use agent_core::prelude::*;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::{Receiver, Sender};

use crate::cel::{Executor, Expression};
use crate::http::Request;
use crate::types::agent::{RouteKey, Target};
tokio::task_local! {
		static ACTIVE: Option<DebugTracer>;
}

pub struct TracingBody(&'static str, RecordedBodyHandle, DebugTracer);

impl TracingBody {
	pub fn maybe_wrap(scope: &'static str, b: Body, limit: usize) -> Body {
		if let Some(tracer) = ACTIVE.try_with(|f| f.clone()).ok().flatten() {
			// RecordBody will get us the Bytes of the request. Note this doesn't block the body, just
			// records.
			let (b, handle) = RecordedBody::new_with_limit(b, limit);
			let t = TracingBody(scope, handle, tracer);
			// Now, we store it in a DropBody so when the body is done we can emit an event.
			DropBody::new(b, t)
		} else {
			b
		}
	}
}

impl Drop for TracingBody {
	fn drop(&mut self) {
		self.2.send(MessageType::BodySnapshot {
			stage: self.0.to_string(),
			body: self.1.bytes(),
		})
	}
}

pub fn is_active() -> bool {
	ACTIVE.try_with(|f| f.is_some()).unwrap_or(false)
}

pub fn trace<F>(f: F)
where
	F: FnOnce(&DebugTracer),
{
	let _ = ACTIVE.try_with(|active| {
		if let Some(a) = active {
			f(a)
		}
	});
}

pub fn with_trace<R>(debug_tracer: Option<DebugTracer>, f: impl FnOnce() -> R) -> R {
	match debug_tracer {
		Some(debug_tracer) => ACTIVE.sync_scope(Some(debug_tracer), f),
		None => f(),
	}
}

pub fn timed_start() -> Option<Instant> {
	is_active().then(Instant::now)
}

pub fn start_scope(name: impl Into<String>) -> ScopeGuard {
	ACTIVE
		.try_with(|active| active.as_ref().map(|trace| trace.start_scope(name.into())))
		.ok()
		.flatten()
		.unwrap_or_else(ScopeGuard::noop)
}

pub fn policy_response_details(pr: &crate::http::PolicyResponse) -> String {
	match pr.direct_response.as_ref() {
		Some(resp) => format!("returned direct response with status {}", resp.status()),
		None => {
			let len = pr
				.response_headers
				.as_ref()
				.map(|h| h.len())
				.unwrap_or_default();
			if len > 0 {
				format!("queued {} response headers", len)
			} else {
				"".to_string()
			}
		},
	}
}

macro_rules! pol_event {
	($severity:expr, $($arg:tt)+) => {{
		tracing::debug!($($arg)+);
		$crate::proxy::dtrace::trace(|trace| {
			trace.policy_event($severity, TRACE_POLICY_KIND, format!($($arg)+))
		});
	}};
	($($arg:tt)+) => {{
		tracing::debug!($($arg)+);
		$crate::proxy::dtrace::trace(|trace| {
			trace.policy_event(Severity::Info, TRACE_POLICY_KIND, format!($($arg)+))
		});
	}};
}

pub(crate) use pol_event;

macro_rules! snapshot {
	(Request, $kind:expr, $request:expr) => {{
		$crate::proxy::dtrace::trace(|trace| {
			trace.request_snapshot(
				$kind,
				crate::cel::Executor::new_request($request).debug_snapshot(),
			)
		});
	}};
	(Response, $kind:expr, $log:expr, $resp:expr) => {{
		$crate::proxy::dtrace::trace(|trace| {
			trace.response_snapshot(
				$kind,
				crate::cel::Executor::new_response($log.request_snapshot.as_deref(), $resp)
					.debug_snapshot(),
			)
		});
	}};
}

pub(crate) use snapshot;

macro_rules! pol_result {
	($kind:expr, $severity:expr, Apply, $($arg:tt)+) => {{
		tracing::debug!($($arg)+);
		$crate::proxy::dtrace::trace(|trace| {
			trace.policy_result(
				$severity,
				$kind,
				$crate::proxy::dtrace::PolicyResult::Apply { details: format!($($arg)+), snapshot: None },
			)
		});
	}};
	($kind:expr, $severity:expr, Skip, $($arg:tt)+) => {{
		tracing::debug!($($arg)+);
		$crate::proxy::dtrace::trace(|trace| {
			trace.policy_result(
				$severity,
				$kind,
				$crate::proxy::dtrace::PolicyResult::Skip { reason: format!($($arg)+) },
			)
		});
	}};
	($kind:expr, $severity:expr, ApplySnapshot, $snapshot:expr, $($arg:tt)+) => {{
		tracing::debug!($($arg)+);
		$crate::proxy::dtrace::trace(|trace| {
			trace.policy_result(
				$severity,
				$kind,
				$crate::proxy::dtrace::PolicyResult::Apply { details: format!($($arg)+), snapshot: Some($snapshot) },
			)
		});
	}};
	($severity:expr, Apply, $($arg:tt)+) => {{
		$crate::proxy::dtrace::pol_result!(TRACE_POLICY_KIND, $severity, Apply, $($arg)+)
	}};
	($severity:expr, Skip, $($arg:tt)+) => {{
		$crate::proxy::dtrace::pol_result!(TRACE_POLICY_KIND, $severity, Skip, $($arg)+)
	}};
	($severity:expr, ApplySnapshot, $snapshot:expr, $($arg:tt)+) => {{
		$crate::proxy::dtrace::pol_result!(TRACE_POLICY_KIND, $severity, ApplySnapshot, $snapshot, $($arg)+)
	}};
}

pub(crate) use pol_result;

macro_rules! pol_result_timed {
	($start:expr, $severity:expr, Apply, $($arg:tt)+) => {{
		let __start = $start;
		tracing::debug!($($arg)+);
		$crate::proxy::dtrace::trace(|trace| {
			let __result = $crate::proxy::dtrace::PolicyResult::Apply {
				details: format!($($arg)+),
				snapshot: None,
			};
			if let Some(__start) = __start {
				trace.policy_result_timed(
					__start,
					::std::time::Instant::now(),
					$severity,
					TRACE_POLICY_KIND,
					__result,
				)
			} else {
				trace.policy_result($severity, TRACE_POLICY_KIND, __result)
			}
		});
	}};
}

pub(crate) use pol_result_timed;

use crate::http::{Body, DropBody, RecordedBody, RecordedBodyHandle};

#[derive(Debug, Serialize)]
#[allow(non_snake_case)]
#[serde(rename_all = "camelCase")]
pub struct Message {
	// Relative time from start, in us
	event_start: Option<u64>,
	event_end: u64,
	severity: Severity,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	scope: Vec<String>,
	message: MessageType,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
	Success,
	Info,
	Warn,
	Error,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationResult {
	Allow,
	Deny,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationRuleMode {
	Allow,
	Deny,
	Require,
}

#[derive(Debug, Serialize)]
#[allow(non_snake_case)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationRuleResult {
	pub name: String,
	pub matched: bool,
	pub mode: AuthorizationRuleMode,
}

#[derive(Debug, Serialize)]
#[allow(non_snake_case)]
#[serde(
	tag = "type",
	rename_all = "camelCase",
	rename_all_fields = "camelCase"
)]
pub enum PolicyResult {
	Skip {
		reason: String,
	},
	Apply {
		details: String,
		snapshot: Option<Value>,
	},
}

#[derive(Debug, Serialize)]
#[allow(non_snake_case)]
#[serde(
	tag = "type",
	rename_all = "camelCase",
	rename_all_fields = "camelCase"
)]
pub enum MessageType {
	RequestStarted,
	Cel {
		expr: String,
		requestState: serde_json::Value,
		result: serde_json::Value,
	},
	RequestSnapshot {
		stage: String,
		requestState: serde_json::Value,
	},
	ResponseSnapshot {
		stage: String,
		requestState: serde_json::Value,
	},
	BodySnapshot {
		stage: String,
		#[serde(serialize_with = "crate::serde_base64::serialize")]
		body: Bytes,
	},
	RouteSelection {
		selectedRoute: Option<RouteKey>,
		evaluatedRoutes: Vec<RouteKey>,
	},
	PolicySelection {
		phase: String,
		effectivePolicy: Value,
	},
	// The final result of a policy evaluation.
	Policy {
		kind: String,
		result: PolicyResult,
	},
	// An event along the way of a policy
	PolicyEvent {
		kind: String,
		details: String,
	},
	AuthorizationResult {
		rules: Vec<AuthorizationRuleResult>,
		result: AuthorizationResult,
	},
	BackendCallStart {
		target: String,
	},
	BackendCallResult {
		status: Option<u16>,
		error: Option<String>,
	},
	LlmRouteResolved {
		provider: String,
		routeType: String,
	},
	LlmRequestDetected {
		provider: String,
		inputFormat: String,
		nativeFormat: Option<String>,
		requestModel: String,
		streaming: bool,
	},
	LlmStreamingTranslation {
		provider: String,
		inputFormat: String,
		nativeFormat: Option<String>,
		streamFormat: String,
	},
	RequestFinished,
}

impl MessageType {
	fn severity(&self) -> Severity {
		match self {
			MessageType::RequestStarted
			| MessageType::RequestSnapshot { .. }
			| MessageType::ResponseSnapshot { .. }
			| MessageType::BodySnapshot { .. }
			| MessageType::RouteSelection {
				selectedRoute: Some(_),
				..
			}
			| MessageType::PolicySelection { .. }
			| MessageType::BackendCallStart { .. }
			| MessageType::LlmRouteResolved { .. }
			| MessageType::LlmRequestDetected { .. }
			| MessageType::LlmStreamingTranslation { .. }
			| MessageType::Policy { .. }
			| MessageType::PolicyEvent { .. }
			| MessageType::RequestFinished => Severity::Info,

			MessageType::AuthorizationResult {
				result: AuthorizationResult::Allow,
				..
			} => Severity::Success,

			MessageType::RouteSelection {
				selectedRoute: None,
				..
			}
			| MessageType::AuthorizationResult {
				result: AuthorizationResult::Deny,
				..
			} => Severity::Error,
			MessageType::Cel { result, .. } => cel_severity(result),
			MessageType::BackendCallResult { status, error, .. } => {
				if error.is_some() || status.is_some_and(|status| status >= 500) {
					Severity::Error
				} else if status.is_some_and(|status| status >= 400) {
					Severity::Warn
				} else {
					Severity::Info
				}
			},
		}
	}
}

fn cel_severity(result: &Value) -> Severity {
	if result.get("error").is_some() {
		Severity::Warn
	} else {
		Severity::Info
	}
}

#[derive(Clone, Debug)]
pub struct DebugTracer {
	sender: tokio::sync::mpsc::Sender<Message>,
	start: Instant,
	scope_state: Arc<Mutex<ScopeState>>,
}

#[derive(Debug)]
struct ScopeState {
	next_id: u64,
	stack: Vec<ScopeFrame>,
}

#[derive(Debug)]
struct ScopeFrame {
	id: u64,
	name: String,
}

#[must_use = "dropping the guard closes the scope"]
pub struct ScopeGuard {
	scope_state: Option<Arc<Mutex<ScopeState>>>,
	id: Option<u64>,
}

impl ScopeGuard {
	fn noop() -> Self {
		Self {
			scope_state: None,
			id: None,
		}
	}
}

struct Watcher {
	id: u64,
	expression: Option<Expression>,
	sender: Sender<Message>,
}

static HAS_WATCHERS: AtomicBool = AtomicBool::new(false);
static NEXT_WATCHER_ID: AtomicU64 = AtomicU64::new(0);
static WATCHERS: Mutex<Vec<Watcher>> = Mutex::new(Vec::new());

pub struct TraceReceiver {
	id: u64,
	receiver: Receiver<Message>,
}

impl TraceReceiver {
	#[cfg(test)]
	pub async fn recv(&mut self) -> Option<Message> {
		self.receiver.recv().await
	}
}

impl tokio_stream::Stream for TraceReceiver {
	type Item = Message;

	fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		let this = self.get_mut();
		Pin::new(&mut this.receiver).poll_recv(cx)
	}
}

impl Drop for TraceReceiver {
	fn drop(&mut self) {
		remove_pending_watcher(self.id);
	}
}

pub fn track_expression(expression: Option<Expression>) -> TraceReceiver {
	let (tx, rx) = tokio::sync::mpsc::channel(32);
	let id = NEXT_WATCHER_ID.fetch_add(1, Ordering::Relaxed);
	let Ok(mut watchers) = WATCHERS.lock() else {
		return TraceReceiver { id, receiver: rx };
	};
	watchers.push(Watcher {
		id,
		expression,
		sender: tx,
	});
	HAS_WATCHERS.store(true, Ordering::Release);
	TraceReceiver { id, receiver: rx }
}

fn remove_pending_watcher(id: u64) {
	let Ok(mut watchers) = WATCHERS.lock() else {
		HAS_WATCHERS.store(false, Ordering::Release);
		return;
	};
	watchers.retain(|watcher| watcher.id != id);
	if watchers.is_empty() {
		HAS_WATCHERS.store(false, Ordering::Release);
	}
}

fn take_sender(req: &Request) -> Option<Sender<Message>> {
	if !HAS_WATCHERS.load(Ordering::Acquire) {
		return None;
	}
	let Ok(mut watchers) = WATCHERS.lock() else {
		HAS_WATCHERS.store(false, Ordering::Release);
		return None;
	};
	watchers.retain(|watcher| !watcher.sender.is_closed());
	if watchers.is_empty() {
		HAS_WATCHERS.store(false, Ordering::Release);
		return None;
	}
	let executor = Executor::new_request(req);
	let index = watchers
		.iter()
		.position(|watcher| match &watcher.expression {
			Some(expression) => executor.eval_bool(expression),
			None => true,
		})?;
	let sender = watchers.remove(index).sender;
	if watchers.is_empty() {
		HAS_WATCHERS.store(false, Ordering::Release);
	}
	Some(sender)
}

impl DebugTracer {
	pub async fn maybe_scope<F, Fut>(req: Request, f: F) -> Fut::Output
	where
		F: FnOnce(Request) -> Fut,
		Fut: Future,
	{
		let Some(tx) = take_sender(&req) else {
			return f(req).await;
		};
		let ins = DebugTracer {
			sender: tx,
			start: Instant::now(),
			scope_state: Arc::new(Mutex::new(ScopeState {
				next_id: 0,
				stack: Vec::new(),
			})),
		};
		ACTIVE.scope(Some(ins), f(req)).await
	}
	pub fn active() -> Option<Self> {
		ACTIVE.try_with(Clone::clone).ok().flatten()
	}
	pub fn start_scope(&self, name: impl Into<String>) -> ScopeGuard {
		let mut scope_state = self.scope_state.lock().expect("scope mutex poisoned");
		let id = scope_state.next_id;
		scope_state.next_id += 1;
		scope_state.stack.push(ScopeFrame {
			id,
			name: name.into(),
		});
		ScopeGuard {
			scope_state: Some(Arc::clone(&self.scope_state)),
			id: Some(id),
		}
	}
	fn current_scope(&self) -> Vec<String> {
		self
			.scope_state
			.lock()
			.expect("scope mutex poisoned")
			.stack
			.iter()
			.map(|frame| frame.name.clone())
			.collect()
	}
	fn send(&self, msg: MessageType) {
		self.send_with_timings(None, Instant::now(), msg)
	}
	fn send_explicit(&self, severity: Severity, msg: MessageType) {
		self.send_explicit_with_timings(None, Instant::now(), severity, msg)
	}
	fn send_with_timings(&self, start: Option<Instant>, end: Instant, msg: MessageType) {
		self.send_explicit_with_timings(start, end, msg.severity(), msg)
	}
	fn send_explicit_with_timings(
		&self,
		start: Option<Instant>,
		end: Instant,
		severity: Severity,
		msg: MessageType,
	) {
		// If the client is disconnected or full then we just drop the events.
		let _ = self.sender.try_send(Message {
			event_start: start.map(|s| u64::try_from((s - self.start).as_micros()).unwrap_or(u64::MAX)),
			event_end: u64::try_from((end - self.start).as_micros()).unwrap_or(u64::MAX),
			severity,
			scope: self.current_scope(),
			message: msg,
		});
	}
	pub fn request_started(&self) {
		self.send(MessageType::RequestStarted)
	}
	pub fn request_completed(&self) {
		self.send(MessageType::RequestFinished)
	}
	pub fn cel_eval(
		&self,
		start: Option<Instant>,
		end: Instant,
		expr: &str,
		data: serde_json::Value,
		result: serde_json::Value,
	) {
		self.send_with_timings(
			start,
			end,
			MessageType::Cel {
				expr: expr.to_string(),
				requestState: data,
				result,
			},
		)
	}
	pub fn request_snapshot(&self, stage: &str, data: Value) {
		self.send(MessageType::RequestSnapshot {
			stage: stage.to_string(),
			requestState: data,
		})
	}
	pub fn response_snapshot(&self, stage: &str, data: Value) {
		self.send(MessageType::ResponseSnapshot {
			stage: stage.to_string(),
			requestState: data,
		})
	}
	pub fn selected_policies(&self, phase: &str, effective_policy: Value) {
		self.send(MessageType::PolicySelection {
			phase: phase.to_string(),
			effectivePolicy: effective_policy,
		})
	}
	pub fn policy_result(&self, severity: Severity, kind: &str, result: PolicyResult) {
		self.send_explicit(
			severity,
			MessageType::Policy {
				kind: kind.to_string(),
				result,
			},
		)
	}
	pub fn policy_result_timed(
		&self,
		start: Instant,
		end: Instant,
		severity: Severity,
		kind: &str,
		result: PolicyResult,
	) {
		self.send_explicit_with_timings(
			Some(start),
			end,
			severity,
			MessageType::Policy {
				kind: kind.to_string(),
				result,
			},
		)
	}
	pub fn policy_event(&self, severity: Severity, kind: &str, details: String) {
		self.send_explicit(
			severity,
			MessageType::PolicyEvent {
				kind: kind.to_string(),
				details,
			},
		)
	}
	pub fn authorization_result(
		&self,
		rules: Vec<AuthorizationRuleResult>,
		result: AuthorizationResult,
	) {
		self.send(MessageType::AuthorizationResult { rules, result })
	}
	pub fn route_selection(&self, selected_route: Option<RouteKey>, evaluated_routes: Vec<RouteKey>) {
		self.send(MessageType::RouteSelection {
			selectedRoute: selected_route,
			evaluatedRoutes: evaluated_routes,
		})
	}
	pub fn backend_call_started(&self, target: &Target) {
		self.send(MessageType::BackendCallStart {
			target: target.to_string(),
		})
	}
	pub fn backend_call_completed(
		&self,
		start: Option<Instant>,
		end: Instant,
		status: Option<u16>,
		error: Option<String>,
	) {
		self.send_with_timings(start, end, MessageType::BackendCallResult { status, error })
	}
	pub fn llm_route_resolved(&self, provider: String, route_type: String) {
		self.send(MessageType::LlmRouteResolved {
			provider,
			routeType: route_type,
		})
	}
	pub fn llm_request_detected(
		&self,
		provider: String,
		input_format: String,
		native_format: Option<String>,
		request_model: String,
		streaming: bool,
	) {
		self.send(MessageType::LlmRequestDetected {
			provider,
			inputFormat: input_format,
			nativeFormat: native_format,
			requestModel: request_model,
			streaming,
		})
	}
	pub fn llm_streaming_translation(
		&self,
		provider: String,
		input_format: String,
		native_format: Option<String>,
		stream_format: String,
	) {
		self.send(MessageType::LlmStreamingTranslation {
			provider,
			inputFormat: input_format,
			nativeFormat: native_format,
			streamFormat: stream_format,
		})
	}
}

impl Drop for ScopeGuard {
	fn drop(&mut self) {
		let Some(scope_state) = self.scope_state.as_ref() else {
			return;
		};
		let Some(id) = self.id.take() else {
			return;
		};
		let mut scope_state = scope_state.lock().expect("scope mutex poisoned");
		if let Some(idx) = scope_state.stack.iter().position(|frame| frame.id == id) {
			scope_state.stack.remove(idx);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::cel::{Executor, Expression};
	use crate::http::Body;

	#[test]
	fn scope_guard_drop_only_removes_its_own_frame() {
		let (tx, _rx) = tokio::sync::mpsc::channel(1);
		let tracer = DebugTracer {
			sender: tx,
			start: Instant::now(),
			scope_state: Arc::new(Mutex::new(ScopeState {
				next_id: 0,
				stack: Vec::new(),
			})),
		};

		let first = tracer.start_scope("first");
		let second = tracer.start_scope("second");

		drop(first);
		assert_eq!(tracer.current_scope(), vec!["second"]);

		drop(second);
		assert!(tracer.current_scope().is_empty());
	}

	#[tokio::test]
	async fn cel_eval_emits_events_while_debug_trace_is_active() {
		let mut trace_rx = track_expression(None);
		let req = http::Request::builder()
			.uri("http://example.com/test")
			.body(Body::empty())
			.expect("request should build");
		let expr = Expression::new_strict("request.path").expect("expression should compile");

		DebugTracer::maybe_scope(req, |req| async move {
			let executor = Executor::new_request(&req);
			let value = executor.eval(&expr).expect("expression should evaluate");
			assert_eq!(value.as_str().unwrap(), "/test");
		})
		.await;

		tokio::time::timeout(Duration::from_secs(1), async {
			while let Some(msg) = trace_rx.recv().await {
				if let MessageType::Cel { expr, result, .. } = msg.message {
					assert_eq!(expr, "request.path");
					assert_eq!(result, serde_json::json!("/test"));
					return;
				}
			}
			panic!("trace receiver closed before CEL trace event was emitted");
		})
		.await
		.expect("trace event should be emitted");
	}

	#[tokio::test]
	async fn cel_eval_emits_events_with_captured_debug_tracer() {
		let (tx, mut rx) = tokio::sync::mpsc::channel(1);
		let tracer = DebugTracer {
			sender: tx,
			start: Instant::now(),
			scope_state: Arc::new(Mutex::new(ScopeState {
				next_id: 0,
				stack: Vec::new(),
			})),
		};
		let req = http::Request::builder()
			.uri("http://example.com/deferred")
			.body(Body::empty())
			.expect("request should build");
		let expr = Expression::new_strict("request.path").expect("expression should compile");

		with_trace(Some(tracer), || {
			let executor = Executor::new_request(&req);
			let value = executor.eval(&expr).expect("expression should evaluate");
			assert_eq!(value.as_str().unwrap(), "/deferred");
		});

		let msg = tokio::time::timeout(Duration::from_secs(1), rx.recv())
			.await
			.expect("trace event should be emitted")
			.expect("trace receiver should still be open");
		match msg.message {
			MessageType::Cel { expr, result, .. } => {
				assert_eq!(expr, "request.path");
				assert_eq!(result, serde_json::json!("/deferred"));
			},
			other => panic!("expected CEL trace event, got {other:?}"),
		}
	}
}
