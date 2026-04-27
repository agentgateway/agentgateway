// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};

use agent_core::drain::DrainWatcher;
use agent_core::readiness;
use hyper::Request;
use hyper::body::Incoming;
use itertools::Itertools;

use super::hyper_helpers;
use crate::Address;
use crate::http::Response;

struct State {
	ready: readiness::Ready,
	not_ready_count: AtomicUsize,
}

impl State {
	fn new(ready: readiness::Ready) -> Self {
		Self {
			ready,
			not_ready_count: AtomicUsize::new(0),
		}
	}
}

pub struct Server {
	s: hyper_helpers::Server<State>,
	ready: readiness::Ready,
}

impl Server {
	pub async fn new(
		address: Address,
		drain_rx: DrainWatcher,
		ready: readiness::Ready,
	) -> anyhow::Result<Self> {
		hyper_helpers::Server::<State>::bind("readiness", address, drain_rx, State::new(ready.clone()))
			.await
			.map(|s| Server {
				s: s.with_optional_proxy_protocol(),
				ready,
			})
	}

	pub fn ready(&self) -> readiness::Ready {
		self.ready.clone()
	}

	pub fn address(&self) -> SocketAddr {
		self.s.address()
	}

	pub fn spawn(self) {
		self.s.spawn(|state, req| async move {
			match req.uri().path() {
				"/healthz/ready" => Ok(handle_ready(&state, req).await),
				_ => Ok(hyper_helpers::empty_response(hyper::StatusCode::NOT_FOUND)),
			}
		})
	}
}

async fn handle_ready(state: &State, req: Request<Incoming>) -> Response {
	match *req.method() {
		hyper::Method::GET => {
			let pending = state.ready.pending();
			if pending.is_empty() {
				state.not_ready_count.store(0, Ordering::Relaxed);
				return hyper_helpers::plaintext_response(hyper::StatusCode::OK, "ready\n".into());
			}

			let attempt = state.not_ready_count.fetch_add(1, Ordering::Relaxed) + 1;
			let pending = pending.into_iter().sorted().join(", ");
			// Users freak out if they see warning logs about "not ready" even when it is expected to happen
			// on startup. Scale up the severity of the logs as we are increasingly not ready.
			match attempt {
				1..=5 => {
					tracing::debug!(attempt, pending, "readiness check failed");
				},
				6..=30 => {
					tracing::info!(attempt, pending, "readiness check failed");
				},
				_ => {
					tracing::warn!(attempt, pending, "readiness check failed");
				},
			}

			hyper_helpers::plaintext_response(
				hyper::StatusCode::INTERNAL_SERVER_ERROR,
				format!("not ready, pending: {pending}\n"),
			)
		},
		_ => hyper_helpers::empty_response(hyper::StatusCode::METHOD_NOT_ALLOWED),
	}
}
