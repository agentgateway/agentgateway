//! Per-target protocol state owned by the MCP relay.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use rmcp::model::{ServerCapabilities, ServerInfo};

#[derive(Debug, Clone)]
pub(crate) struct TargetSession {
	info: Arc<RwLock<Option<ServerInfo>>>,
	pending_elicitations: Arc<RwLock<HashMap<String, String>>>,
}

impl TargetSession {
	pub(crate) fn new() -> Self {
		Self {
			info: Arc::new(RwLock::new(None)),
			pending_elicitations: Arc::new(RwLock::new(HashMap::new())),
		}
	}

	pub(crate) fn info(&self) -> Option<ServerInfo> {
		let info = self.info.read().unwrap_or_else(|e| {
			tracing::error!("target session info lock poisoned while reading; continuing");
			e.into_inner()
		});
		info.clone()
	}

	pub(crate) fn set_info(&self, info: ServerInfo) {
		let mut slot = self.info.write().unwrap_or_else(|e| {
			tracing::error!("target session info lock poisoned while updating; continuing");
			e.into_inner()
		});
		*slot = Some(info);
	}

	pub(crate) fn supports_capability(
		&self,
		check: impl Fn(&ServerCapabilities) -> bool,
	) -> Option<bool> {
		self.info().map(|info| check(&info.capabilities))
	}

	pub(crate) fn register_pending_elicitation(&self, routed_id: &str, original_id: &str) {
		let mut pending = self.pending_elicitations.write().unwrap_or_else(|e| {
			tracing::error!(
				"target session pending elicitation lock poisoned while registering; continuing"
			);
			e.into_inner()
		});
		pending.insert(routed_id.to_string(), original_id.to_string());
	}

	pub(crate) fn clear_pending_elicitation(&self, original_id: &str) {
		let mut pending = self.pending_elicitations.write().unwrap_or_else(|e| {
			tracing::error!(
				"target session pending elicitation lock poisoned while clearing; continuing"
			);
			e.into_inner()
		});
		pending.retain(|_, active_original_id| active_original_id != original_id);
	}

	pub(crate) fn take_active_elicitation(&self, routed_id: &str, original_id: &str) -> bool {
		let mut pending = self.pending_elicitations.write().unwrap_or_else(|e| {
			tracing::error!(
				"target session pending elicitation lock poisoned while consuming; continuing"
			);
			e.into_inner()
		});
		pending
			.remove(routed_id)
			.is_some_and(|active_original_id| active_original_id == original_id)
	}
}
