//! Retries for policy callouts (guardrails, ext_authz, webhooks, ...) made through `PolicyClient`.
//!
//! The retry policy is attached as an inline backend policy of the callout. Retries nest with any
//! route retry: a retried route request re-runs its callouts, and each callout retries on its own.
//! One deadline bounds all attempts and backoffs of a callout.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, warn};

use crate::http::filters::BackendRequestTimeout;
use crate::http::{Body, Request, Response, retry};
use crate::proxy::ProxyError;
use crate::proxy::httpproxy::PolicyClient;
use crate::store::BackendPolicies;
use crate::types::agent::Backend;
use crate::*;

/// Deadline for a retried callout when neither the caller nor `http.requestTimeout` sets one.
pub(super) const DEFAULT_CALLOUT_DEADLINE: Duration = Duration::from_secs(10);

/// Extra retry check a caller can attach to a callout request. A response is retried when this
/// returns true, in addition to `codes` and `condition`. Useful for providers that report
/// retryable errors in the body of a 200.
#[derive(Clone)]
pub struct CalloutRetryClassifier(pub Arc<dyn Fn(&Response) -> bool + Send + Sync>);

impl CalloutRetryClassifier {
	pub fn new(f: impl Fn(&Response) -> bool + Send + Sync + 'static) -> Self {
		Self(Arc::new(f))
	}
}

/// Runs a callout under its retry policy. Each attempt is a single call, and the request timeout
/// (backend `http.requestTimeout`, the caller's timeout, or 10s) is the deadline for all of them.
pub(super) async fn call(
	client: &PolicyClient,
	req: Request,
	backend: Backend,
	mut pols: BackendPolicies,
	pol: Arc<retry::Policy>,
) -> Result<Response, ProxyError> {
	// Attempts must not recurse into another retry loop.
	pols.retry = None;
	let deadline = pols
		.http
		.as_ref()
		.and_then(|h| h.request_timeout)
		.or_else(|| req.extensions().get::<BackendRequestTimeout>().map(|t| t.0))
		.unwrap_or(DEFAULT_CALLOUT_DEADLINE);
	let classifier = req.extensions().get::<CalloutRetryClassifier>().cloned();
	with_deadline(deadline, &pol, classifier.as_ref(), req, |req| {
		client.internal_call_with_policies(req, backend.clone(), pols.clone())
	})
	.await
}

pub(super) async fn with_deadline<F, Fut>(
	deadline: Duration,
	pol: &retry::Policy,
	classifier: Option<&CalloutRetryClassifier>,
	req: Request,
	attempt: F,
) -> Result<Response, ProxyError>
where
	F: FnMut(Request) -> Fut,
	Fut: Future<Output = Result<Response, ProxyError>>,
{
	let deadline = tokio::time::Instant::now() + deadline;
	let mut res = tokio::time::timeout_at(deadline, retry_loop(pol, classifier, req, attempt))
		.await
		.unwrap_or(Err(ProxyError::UpstreamCallTimeout));
	// Each attempt sets its own body deadline; keep the returned body within the shared one.
	if let Ok(resp) = &mut res
		&& resp.body().deadline().is_none_or(|d| d > deadline)
	{
		resp.body_mut().set_deadline(deadline);
	}
	res
}

async fn retry_loop<F, Fut>(
	pol: &retry::Policy,
	classifier: Option<&CalloutRetryClassifier>,
	req: Request,
	mut attempt: F,
) -> Result<Response, ProxyError>
where
	F: FnMut(Request) -> Fut,
	Fut: Future<Output = Result<Response, ProxyError>>,
{
	if pol.attempts == 0 || !pol.matches_request(&req) {
		return attempt(req).await;
	}
	// Callout bodies are built in memory; only buffer when the size is known to fit.
	// gRPC and streaming callouts have no known size, so they are never retried.
	let limit = crate::http::buffer_limit(&req);
	let fits = http_body::Body::size_hint(req.body())
		.upper()
		.is_some_and(|n| n <= limit as u64);
	if !fits {
		static WARNED: std::sync::Once = std::sync::Once::new();
		WARNED.call_once(|| {
			warn!("callout body has no known size within the buffer limit, retry is disabled")
		});
		debug!("callout body has no known size within the buffer limit, retry is disabled");
		return attempt(req).await;
	}
	let (head, body) = req.into_parts();
	let body = crate::http::read_body_with_limit(body, limit)
		.await
		.map_err(|e| ProxyError::Processing(anyhow::anyhow!("failed to buffer callout body: {e}")))?;

	let snapshot = pol.condition.as_ref().map(|_| {
		let mut req = Request::from_parts(head.clone(), Body::from(body.clone()));
		cel::snapshot_request(&mut req, false)
	});
	let inspect_body = classifier.is_some()
		|| pol
			.condition
			.as_ref()
			.is_some_and(|c| c.attributes().contains(cel::Attributes::ResponseBody));
	let mut n = 0;
	loop {
		let mut res = attempt(Request::from_parts(head.clone(), Body::from(body.clone()))).await;
		if n == pol.attempts
			|| !should_retry(&mut res, pol, snapshot.as_ref(), classifier, inspect_body).await
		{
			return res;
		}
		n += 1;
		debug!(
			attempt = n,
			backoff = ?pol.backoff,
			"retrying callout, last result was {:?}",
			res.as_ref().map(|r| r.status())
		);
		if let Some(backoff) = pol.backoff {
			tokio::time::sleep(backoff).await;
		}
	}
}

/// Status codes are checked before the body is read, and the body is inspected only when the
/// condition or classifier needs it. A body over the buffer limit is still returned intact.
async fn should_retry(
	res: &mut Result<Response, ProxyError>,
	pol: &retry::Policy,
	snapshot: Option<&cel::RequestSnapshot>,
	classifier: Option<&CalloutRetryClassifier>,
	inspect_body: bool,
) -> bool {
	let resp = match res {
		Ok(resp) => resp,
		Err(e) => return e.is_retryable(),
	};
	if pol.codes.contains(&resp.status()) {
		return true;
	}
	if inspect_body {
		resp.body_mut().require_inspection();
		if let Err(e) = crate::http::inspect_response_body(resp).await {
			debug!("failed to read callout response for retry: {e}");
		}
	}
	pol.matches_response(snapshot, resp) || classifier.is_some_and(|c| (c.0)(resp))
}

#[cfg(test)]
mod tests {
	use std::sync::Mutex;

	use bytes::Bytes;

	use super::*;

	fn policy(attempts: u8, codes: &[u16], condition: Option<&str>) -> retry::Policy {
		retry::Policy {
			attempts,
			backoff: None,
			codes: codes
				.iter()
				.map(|c| ::http::StatusCode::from_u16(*c).unwrap())
				.collect(),
			precondition: None,
			condition: condition.map(|e| Arc::new(cel::Expression::new_strict(e).unwrap())),
		}
	}

	fn request() -> Request {
		::http::Request::builder()
			.method("POST")
			.uri("http://callout.example/check")
			.body(Body::from("payload"))
			.unwrap()
	}

	fn status(code: u16) -> Result<Response, ProxyError> {
		Ok(
			::http::Response::builder()
				.status(code)
				.body(Body::empty())
				.unwrap(),
		)
	}

	fn json(code: u16, body: &'static str) -> Result<Response, ProxyError> {
		Ok(
			::http::Response::builder()
				.status(code)
				.body(Body::from(body))
				.unwrap(),
		)
	}

	async fn body_of(res: Result<Response, ProxyError>) -> Bytes {
		crate::http::read_body_with_limit(res.unwrap().into_body(), 1024)
			.await
			.unwrap()
	}

	/// Runs the loop against a fake callout answering with `results` in order.
	/// Returns the outcome and the body of every attempt.
	async fn run(
		deadline: Duration,
		pol: retry::Policy,
		results: Vec<Result<Response, ProxyError>>,
	) -> (Result<Response, ProxyError>, Vec<Bytes>) {
		run_with(deadline, pol, None, results).await
	}

	async fn run_with(
		deadline: Duration,
		pol: retry::Policy,
		classifier: Option<CalloutRetryClassifier>,
		results: Vec<Result<Response, ProxyError>>,
	) -> (Result<Response, ProxyError>, Vec<Bytes>) {
		let results = Mutex::new(results.into_iter());
		let sent = Mutex::new(Vec::new());
		let res = with_deadline(deadline, &pol, classifier.as_ref(), request(), |req| {
			let next = results
				.lock()
				.unwrap()
				.next()
				.expect("unexpected extra attempt");
			let sent = &sent;
			async move {
				let body = crate::http::read_body_with_limit(req.into_body(), 1024)
					.await
					.unwrap();
				sent.lock().unwrap().push(body);
				next
			}
		})
		.await;
		(res, sent.into_inner().unwrap())
	}

	#[tokio::test]
	async fn retries_listed_codes_and_connection_errors_only() {
		let ten = DEFAULT_CALLOUT_DEADLINE;
		// (callout results, final status or None for an error, attempts made)
		type Case = (Vec<Result<Response, ProxyError>>, Option<u16>, usize);
		let cases: Vec<Case> = vec![
			(vec![status(503), status(200)], Some(200), 2),
			(
				vec![Err(ProxyError::UpstreamCallTimeout), status(200)],
				Some(200),
				2,
			),
			(vec![status(400)], Some(400), 1),
			(vec![Err(ProxyError::InvalidRequest)], None, 1),
			// Retries exhausted: the last response is returned as is.
			(vec![status(503), status(503), status(503)], Some(503), 3),
		];
		for (results, want, calls) in cases {
			let (res, sent) = run(ten, policy(2, &[503], None), results).await;
			assert_eq!(res.as_ref().ok().map(|r| r.status().as_u16()), want);
			assert_eq!(sent.len(), calls);
			assert!(
				sent.iter().all(|b| b == "payload"),
				"every attempt resends the body"
			);
		}
	}

	#[tokio::test]
	async fn zero_attempts_disables_retries() {
		let (res, sent) = run(
			DEFAULT_CALLOUT_DEADLINE,
			policy(0, &[503], None),
			vec![status(503)],
		)
		.await;
		assert_eq!(res.unwrap().status(), 503);
		assert_eq!(sent.len(), 1);
	}

	#[tokio::test]
	async fn condition_retries_matching_responses() {
		let pol = policy(
			1,
			&[],
			Some(r#"response.code == 200 && response.headers["x-transient"] == "true""#),
		);
		let transient = ::http::Response::builder()
			.status(200)
			.header("x-transient", "true")
			.body(Body::empty())
			.unwrap();
		let (res, sent) = run(
			DEFAULT_CALLOUT_DEADLINE,
			pol,
			vec![Ok(transient), status(200)],
		)
		.await;
		assert_eq!(res.unwrap().status(), 200);
		assert_eq!(sent.len(), 2);
	}

	#[tokio::test]
	async fn condition_reads_response_body() {
		let pol = policy(1, &[], Some("json(response.body).error.retryable"));
		let (res, sent) = run(
			DEFAULT_CALLOUT_DEADLINE,
			pol,
			vec![
				json(200, r#"{"error":{"retryable":true}}"#),
				json(200, r#"{"error":{"retryable":false}}"#),
			],
		)
		.await;
		assert_eq!(sent.len(), 2);
		assert_eq!(body_of(res).await, r#"{"error":{"retryable":false}}"#);
	}

	#[tokio::test]
	async fn condition_sees_callout_request() {
		let pol = policy(
			1,
			&[],
			Some(r#"request.method == "POST" && response.code == 503"#),
		);
		let (res, sent) = run(
			DEFAULT_CALLOUT_DEADLINE,
			pol,
			vec![status(503), status(200)],
		)
		.await;
		assert_eq!(res.unwrap().status(), 200);
		assert_eq!(sent.len(), 2);
	}

	fn oversized(code: u16) -> Result<Response, ProxyError> {
		let mut resp = ::http::Response::builder()
			.status(code)
			.body(Body::from("too large for the limit"))
			.unwrap();
		resp
			.extensions_mut()
			.insert(crate::http::BufferLimit::new(4));
		Ok(resp)
	}

	#[tokio::test]
	async fn oversized_responses_are_retried_by_code_and_kept_intact() {
		let classifier = CalloutRetryClassifier::new(|resp| {
			resp
				.body()
				.known_bytes()
				.is_some_and(|b| b.as_ref() == b"transient")
		});
		let (res, sent) = run_with(
			DEFAULT_CALLOUT_DEADLINE,
			policy(2, &[503], Some("json(response.body).retryable")),
			Some(classifier),
			vec![oversized(503), oversized(200)],
		)
		.await;
		assert_eq!(sent.len(), 2);
		assert_eq!(body_of(res).await, "too large for the limit");
	}

	#[tokio::test]
	async fn classifier_adds_to_codes() {
		let classifier = CalloutRetryClassifier::new(|resp| {
			resp
				.body()
				.known_bytes()
				.is_some_and(|b| b.as_ref() == b"transient")
		});
		let (res, sent) = run_with(
			DEFAULT_CALLOUT_DEADLINE,
			policy(2, &[503], None),
			Some(classifier),
			vec![json(200, "transient"), status(503), json(200, "ok")],
		)
		.await;
		assert_eq!(sent.len(), 3);
		assert_eq!(body_of(res).await, "ok");
	}

	#[tokio::test]
	async fn policy_client_applies_inline_retry_and_classifier() {
		use wiremock::matchers::method;
		use wiremock::{Mock, MockServer, ResponseTemplate};

		use crate::types::agent::{BackendTrafficPolicy, ResourceName, Target};

		let server = MockServer::start().await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(200).set_body_string("transient"))
			.up_to_n_times(1)
			.mount(&server)
			.await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(200).set_body_string("ok"))
			.mount(&server)
			.await;

		let client = crate::test_helpers::policy_client();
		let backend = Backend::Opaque(
			ResourceName::new(strng::literal!("callout"), strng::literal!("")),
			Target::Address(*server.address()),
		);
		let mut req = request();
		req
			.extensions_mut()
			.insert(CalloutRetryClassifier::new(|resp| {
				resp
					.body()
					.known_bytes()
					.is_some_and(|b| b.as_ref() == b"transient")
			}));
		let res = client
			.call_with_explicit_policies_list(
				req,
				backend,
				vec![BackendTrafficPolicy::Retry(Arc::new(policy(1, &[], None)))],
			)
			.await;
		assert_eq!(body_of(res).await, "ok");
		let received = server.received_requests().await.unwrap();
		assert_eq!(received.len(), 2);
		assert!(received.iter().all(|r| r.body == b"payload"));
	}

	#[tokio::test]
	async fn primary_traffic_is_not_retried() {
		use wiremock::matchers::method;
		use wiremock::{Mock, MockServer, ResponseTemplate};

		use crate::telemetry::metrics::{OutboundCallKind, OutboundCallSubtype};
		use crate::types::agent::{BackendTrafficPolicy, ResourceName, Target};

		let server = MockServer::start().await;
		Mock::given(method("POST"))
			.respond_with(ResponseTemplate::new(503))
			.mount(&server)
			.await;

		let client = crate::test_helpers::policy_client()
			.with_outbound(OutboundCallKind::Primary, OutboundCallSubtype::Mcp);
		let backend = Backend::Opaque(
			ResourceName::new(strng::literal!("mcp"), strng::literal!("")),
			Target::Address(*server.address()),
		);
		let res = client
			.call_with_explicit_policies_list(
				request(),
				backend,
				vec![BackendTrafficPolicy::Retry(Arc::new(policy(
					2,
					&[503],
					None,
				)))],
			)
			.await;
		assert_eq!(res.unwrap().status(), 503);
		assert_eq!(server.received_requests().await.unwrap().len(), 1);
	}

	#[tokio::test(start_paused = true)]
	async fn returned_body_deadline_is_capped_at_shared_deadline() {
		let start = tokio::time::Instant::now();
		let attempts = std::sync::atomic::AtomicUsize::new(0);
		let res = with_deadline(
			DEFAULT_CALLOUT_DEADLINE,
			&policy(1, &[503], None),
			None,
			request(),
			|_req| {
				let first = attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
				async move {
					if first {
						tokio::time::sleep(Duration::from_secs(8)).await;
						return status(503);
					}
					// Like the HTTP client, the attempt's body deadline starts at the attempt.
					let mut resp = status(200)?;
					resp
						.body_mut()
						.set_deadline(tokio::time::Instant::now() + DEFAULT_CALLOUT_DEADLINE);
					Ok(resp)
				}
			},
		)
		.await
		.unwrap();
		assert_eq!(
			res.body().deadline(),
			Some(start + DEFAULT_CALLOUT_DEADLINE)
		);
	}

	#[tokio::test(start_paused = true)]
	async fn deadline_bounds_all_attempts_and_backoff() {
		let mut pol = policy(5, &[503], None);
		pol.backoff = Some(Duration::from_secs(2));
		let attempts = std::sync::atomic::AtomicUsize::new(0);
		let start = tokio::time::Instant::now();
		// Each attempt takes 4s: 0-4s, backoff to 6s, 6-10s hits the 10s deadline.
		let res = with_deadline(DEFAULT_CALLOUT_DEADLINE, &pol, None, request(), |_req| {
			attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
			async {
				tokio::time::sleep(Duration::from_secs(4)).await;
				status(503)
			}
		})
		.await;
		assert!(matches!(res, Err(ProxyError::UpstreamCallTimeout)));
		assert_eq!(attempts.into_inner(), 2);
		assert_eq!(start.elapsed(), DEFAULT_CALLOUT_DEADLINE);
	}
}
