// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use tokio::sync::watch;
use tracing::info;

use crate::telemetry;

#[derive(Debug)]
struct ReadyState {
	pending: HashSet<String>,
	ready_tx: watch::Sender<bool>,
}

/// Ready tracks whether the process is ready.
#[derive(Clone, Debug)]
pub struct Ready(Arc<Mutex<ReadyState>>);

impl Default for Ready {
	fn default() -> Self {
		Self::new()
	}
}

impl Ready {
	pub fn new() -> Ready {
		let (ready_tx, _ready_rx) = watch::channel(false);
		Ready(Arc::new(Mutex::new(ReadyState {
			pending: Default::default(),
			ready_tx,
		})))
	}

	/// register_task allows a caller to add a dependency to be marked "ready".
	pub fn register_task(&self, name: &str) -> BlockReady {
		let mut state = self.0.lock().unwrap();
		let was_ready = state.pending.is_empty();
		state.pending.insert(name.to_string());
		if was_ready {
			state.ready_tx.send_replace(false);
		}
		BlockReady {
			parent: self.to_owned(),
			name: name.to_string(),
		}
	}

	pub fn pending(&self) -> HashSet<String> {
		self.0.lock().unwrap().pending.clone()
	}

	pub fn subscribe(&self) -> watch::Receiver<bool> {
		self.0.lock().unwrap().ready_tx.subscribe()
	}
}

/// BlockReady blocks readiness until it is dropped.
pub struct BlockReady {
	parent: Ready,
	name: String,
}

impl BlockReady {
	pub fn subtask(&self, name: &str) -> BlockReady {
		self.parent.register_task(name)
	}
}

impl Drop for BlockReady {
	fn drop(&mut self) {
		let mut state = self.parent.0.lock().unwrap();
		let removed = state.pending.remove(&self.name);
		debug_assert!(removed); // It is a bug to somehow remove something twice
		let left = state.pending.len();
		let dur = telemetry::APPLICATION_START_TIME.elapsed();
		if left == 0 {
			state.ready_tx.send_replace(true);
			info!(
				"Task '{}' complete ({dur:?}), marking server ready",
				self.name
			);
		} else {
			info!(
				"Task '{}' complete ({dur:?}), still awaiting {left} tasks",
				self.name
			);
		}
	}
}
