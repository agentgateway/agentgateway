//! Tool cycle validation for Anthropic → Bedrock

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Conversation identifier - should be stable across turns
pub type ConversationId = String;

/// A pending tool cycle tracking unfulfilled tool use IDs
#[derive(Debug, Clone)]
pub struct PendingToolCycle {
	/// IDs still waiting for tool results
	pub tool_use_ids: HashSet<String>,
	/// Original set of IDs (for duplicate detection)
	pub original_ids: HashSet<String>,
	/// When this cycle was created
	pub created: Instant,
}

impl PendingToolCycle {
	/// Create new tool cycle from iterator of tool use IDs
	pub fn new<I: IntoIterator<Item = String>>(ids: I) -> Self {
		let set: HashSet<String> = ids.into_iter().collect();
		Self {
			tool_use_ids: set.clone(),
			original_ids: set,
			created: Instant::now(),
		}
	}

	/// Check if all tool uses have been fulfilled
	pub fn is_fulfilled(&self) -> bool {
		self.tool_use_ids.is_empty()
	}
}

/// Errors that can occur during tool cycle fulfillment
#[derive(Debug, PartialEq, Eq)]
pub enum FulfillmentError {
	/// Tool result ID not found in pending set
	UnexpectedId(String),
	/// Some tool use IDs still missing results
	MissingIds(Vec<String>),
	/// No pending tool cycle for this conversation
	NoPendingCycle,
}

impl std::fmt::Display for FulfillmentError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			FulfillmentError::UnexpectedId(id) => {
				write!(f, "Unexpected tool result ID: {}", id)
			},
			FulfillmentError::MissingIds(ids) => {
				write!(f, "Missing tool result IDs: {}", ids.join(", "))
			},
			FulfillmentError::NoPendingCycle => {
				write!(f, "No pending tool cycle for conversation")
			},
		}
	}
}

impl std::error::Error for FulfillmentError {}

/// Result of partial fulfillment operation
#[derive(Debug, Clone)]
pub struct PartialFulfillment {
	/// Whether the cycle is now complete
	pub complete: bool,
	/// Remaining unfulfilled tool use IDs
	pub remaining: Vec<String>,
}

/// Comprehensive metrics for tool cycle operations
#[derive(Debug, Clone, Default)]
pub struct ToolCycleMetrics {
	/// Total cycles stored
	pub stored: u64,
	/// Total cycles successfully fulfilled
	pub fulfilled: u64,
	/// Rejections due to unexpected tool result IDs
	pub rejected_unexpected: u64,
	/// Rejections due to missing tool results
	pub rejected_missing: u64,
	/// Cycles expired due to TTL
	pub expired: u64,
	/// Cycles evicted due to capacity limits
	pub evicted_capacity: u64,
	/// Duplicate/idempotent tool result attempts
	pub duplicate_idempotent: u64,
}

/// Internal store state
#[derive(Debug, Default)]
struct StoreInner {
	cycles: HashMap<ConversationId, PendingToolCycle>,
}

/// Configuration for tool cycle store
#[derive(Debug, Clone)]
pub struct ToolCycleConfig {
	/// Time-to-live for pending cycles (default: 5 minutes)
	pub ttl_seconds: u64,
	/// Maximum number of active cycles (default: 5000)  
	pub max_active_cycles: usize,
	/// Whether validation is enabled (default: true)
	pub enabled: bool,
}

impl Default for ToolCycleConfig {
	fn default() -> Self {
		Self {
			ttl_seconds: 300, // 5 minutes
			max_active_cycles: 5000,
			enabled: true,
		}
	}
}

/// Thread-safe tool cycle validation store
pub struct ToolCycleStore {
	inner: Arc<Mutex<StoreInner>>,
	config: Arc<Mutex<ToolCycleConfig>>,

	// Atomic counters for metrics
	stored: AtomicU64,
	fulfilled: AtomicU64,
	rejected_unexpected: AtomicU64,
	rejected_missing: AtomicU64,
	expired: AtomicU64,
	evicted_capacity: AtomicU64,
	duplicate_idempotent: AtomicU64,
}

impl Default for ToolCycleStore {
	fn default() -> Self {
		Self::new(ToolCycleConfig::default())
	}
}

impl ToolCycleStore {
	/// Create new tool cycle store with configuration
	pub fn new(config: ToolCycleConfig) -> Self {
		Self {
			inner: Arc::new(Mutex::new(StoreInner::default())),
			config: Arc::new(Mutex::new(config)),
			stored: AtomicU64::new(0),
			fulfilled: AtomicU64::new(0),
			rejected_unexpected: AtomicU64::new(0),
			rejected_missing: AtomicU64::new(0),
			expired: AtomicU64::new(0),
			evicted_capacity: AtomicU64::new(0),
			duplicate_idempotent: AtomicU64::new(0),
		}
	}

	/// Update store configuration
	pub fn update_config(&self, config: ToolCycleConfig) {
		if let Ok(mut guard) = self.config.lock() {
			*guard = config;
			debug!("Tool cycle store configuration updated");
		}
	}

	/// Insert new tool cycle for tracking
	pub fn insert_cycle(&self, conversation_id: &str, tool_use_ids: Vec<String>) {
		let config = if let Ok(guard) = self.config.lock() {
			guard.clone()
		} else {
			return;
		};

		if !config.enabled || tool_use_ids.is_empty() {
			return;
		}

		if let Ok(mut inner) = self.inner.lock() {
			// Enforce capacity limits by evicting oldest cycles
			while inner.cycles.len() >= config.max_active_cycles {
				if let Some((oldest_key, _)) = inner
					.cycles
					.iter()
					.min_by_key(|(_, cycle)| cycle.created)
					.map(|(k, v)| (k.clone(), v.created))
				{
					inner.cycles.remove(&oldest_key);
					self.evicted_capacity.fetch_add(1, Ordering::Relaxed);
					warn!(
							conversation_id = %oldest_key,
							"Tool cycle evicted due to capacity limit"
					);
				} else {
					break;
				}
			}

			// Insert new cycle
			inner.cycles.insert(
				conversation_id.to_string(),
				PendingToolCycle::new(tool_use_ids.clone()),
			);

			self.stored.fetch_add(1, Ordering::Relaxed);
			debug!(
					conversation_id = %conversation_id,
					tool_use_count = tool_use_ids.len(),
					"Tool cycle stored"
			);
		}
	}

	/// Remove and return tool cycle for conversation
	pub fn take_cycle(&self, conversation_id: &str) -> Option<PendingToolCycle> {
		if let Ok(mut inner) = self.inner.lock() {
			inner.cycles.remove(conversation_id)
		} else {
			None
		}
	}

	/// Peek at pending tool use IDs without removing cycle
	pub fn peek_pending_ids(&self, conversation_id: &str) -> Option<Vec<String>> {
		if let Ok(inner) = self.inner.lock() {
			inner
				.cycles
				.get(conversation_id)
				.map(|cycle| cycle.tool_use_ids.iter().cloned().collect())
		} else {
			None
		}
	}

	/// Fulfill tool cycle completely - all tool results must be provided
	pub fn fulfill_complete(
		&self,
		conversation_id: &str,
		tool_result_ids: &[String],
	) -> Result<(), FulfillmentError> {
		let config = if let Ok(guard) = self.config.lock() {
			guard.clone()
		} else {
			return Ok(()); // If config lock fails, allow request through
		};

		if !config.enabled {
			return Ok(());
		}

		if let Ok(mut inner) = self.inner.lock() {
			if let Some(cycle) = inner.cycles.get_mut(conversation_id) {
				// Check for unexpected IDs
				for result_id in tool_result_ids {
					if !cycle.tool_use_ids.remove(result_id) {
						// Check if this was part of original set (idempotent retry)
						if cycle.original_ids.contains(result_id) {
							self.duplicate_idempotent.fetch_add(1, Ordering::Relaxed);
							debug!(
									conversation_id = %conversation_id,
									tool_result_id = %result_id,
									"Duplicate idempotent tool result ignored"
							);
							continue;
						}

						self.rejected_unexpected.fetch_add(1, Ordering::Relaxed);
						return Err(FulfillmentError::UnexpectedId(result_id.clone()));
					}
				}

				// Check if all tool uses are fulfilled
				if !cycle.tool_use_ids.is_empty() {
					let missing: Vec<String> = cycle.tool_use_ids.iter().cloned().collect();
					self.rejected_missing.fetch_add(1, Ordering::Relaxed);
					return Err(FulfillmentError::MissingIds(missing));
				}

				// Complete fulfillment - remove cycle
				inner.cycles.remove(conversation_id);
				self.fulfilled.fetch_add(1, Ordering::Relaxed);
				debug!(
						conversation_id = %conversation_id,
						"Tool cycle completely fulfilled"
				);

				Ok(())
			} else {
				self.rejected_missing.fetch_add(1, Ordering::Relaxed);
				Err(FulfillmentError::NoPendingCycle)
			}
		} else {
			// If lock fails, allow request through (fail open)
			Ok(())
		}
	}

	/// Fulfill tool cycle partially - update tracking but don't require completion
	pub fn fulfill_partial(
		&self,
		conversation_id: &str,
		tool_result_ids: &[String],
	) -> Result<PartialFulfillment, FulfillmentError> {
		let config = if let Ok(guard) = self.config.lock() {
			guard.clone()
		} else {
			return Ok(PartialFulfillment {
				complete: true,
				remaining: vec![],
			});
		};

		if !config.enabled {
			return Ok(PartialFulfillment {
				complete: true,
				remaining: vec![],
			});
		}

		if let Ok(mut inner) = self.inner.lock() {
			if let Some(cycle) = inner.cycles.get_mut(conversation_id) {
				// Process provided tool result IDs
				for result_id in tool_result_ids {
					if !cycle.tool_use_ids.remove(result_id) {
						// Check for idempotent retry
						if cycle.original_ids.contains(result_id) {
							self.duplicate_idempotent.fetch_add(1, Ordering::Relaxed);
							continue;
						}

						self.rejected_unexpected.fetch_add(1, Ordering::Relaxed);
						return Err(FulfillmentError::UnexpectedId(result_id.clone()));
					}
				}

				// Check completion status
				if cycle.tool_use_ids.is_empty() {
					inner.cycles.remove(conversation_id);
					self.fulfilled.fetch_add(1, Ordering::Relaxed);

					Ok(PartialFulfillment {
						complete: true,
						remaining: vec![],
					})
				} else {
					let remaining: Vec<String> = cycle.tool_use_ids.iter().cloned().collect();

					Ok(PartialFulfillment {
						complete: false,
						remaining,
					})
				}
			} else {
				Err(FulfillmentError::NoPendingCycle)
			}
		} else {
			// If lock fails, report as complete (fail open)
			Ok(PartialFulfillment {
				complete: true,
				remaining: vec![],
			})
		}
	}

	/// Garbage collect expired cycles
	pub fn garbage_collect(&self) {
		let config = if let Ok(guard) = self.config.lock() {
			guard.clone()
		} else {
			return;
		};

		if let Ok(mut inner) = self.inner.lock() {
			let ttl = Duration::from_secs(config.ttl_seconds);
			let before_count = inner.cycles.len();

			inner
				.cycles
				.retain(|_, cycle| cycle.created.elapsed() < ttl);

			let expired_count = before_count - inner.cycles.len();
			if expired_count > 0 {
				self
					.expired
					.fetch_add(expired_count as u64, Ordering::Relaxed);
				debug!(
					expired_cycles = expired_count,
					"Expired tool cycles cleaned up"
				);
			}
		}
	}

	/// Get current metrics snapshot
	pub fn metrics(&self) -> ToolCycleMetrics {
		ToolCycleMetrics {
			stored: self.stored.load(Ordering::Relaxed),
			fulfilled: self.fulfilled.load(Ordering::Relaxed),
			rejected_unexpected: self.rejected_unexpected.load(Ordering::Relaxed),
			rejected_missing: self.rejected_missing.load(Ordering::Relaxed),
			expired: self.expired.load(Ordering::Relaxed),
			evicted_capacity: self.evicted_capacity.load(Ordering::Relaxed),
			duplicate_idempotent: self.duplicate_idempotent.load(Ordering::Relaxed),
		}
	}

	/// Get count of currently active cycles
	pub fn active_cycle_count(&self) -> usize {
		if let Ok(inner) = self.inner.lock() {
			inner.cycles.len()
		} else {
			0
		}
	}
}

/// Global tool cycle store instance
static GLOBAL_TOOL_CYCLE_STORE: std::sync::OnceLock<ToolCycleStore> = std::sync::OnceLock::new();

/// Get global tool cycle store instance
pub fn global_tool_cycle_store() -> &'static ToolCycleStore {
	GLOBAL_TOOL_CYCLE_STORE.get_or_init(|| ToolCycleStore::default())
}

/// Extract tool use IDs from Anthropic content blocks
pub fn extract_tool_use_ids(
	content: &[crate::llm::anthropic_types::ResponseContentBlock],
) -> Vec<String> {
	content
		.iter()
		.filter_map(|block| match block {
			crate::llm::anthropic_types::ResponseContentBlock::ToolUse(tool_use) => {
				Some(tool_use.id.clone())
			},
			_ => None,
		})
		.collect()
}

/// Extract tool result IDs from Anthropic content blocks
pub fn extract_tool_result_ids(
	content: &[crate::llm::anthropic_types::RequestContentBlock],
) -> Vec<String> {
	content
		.iter()
		.filter_map(|block| match block {
			crate::llm::anthropic_types::RequestContentBlock::ToolResult(tool_result) => {
				Some(tool_result.tool_use_id.clone())
			},
			_ => None,
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_tool_cycle_basic_flow() {
		let store = ToolCycleStore::default();
		let conv_id = "test-conversation";
		let tool_ids = vec!["tool1".to_string(), "tool2".to_string()];

		// Insert cycle
		store.insert_cycle(conv_id, tool_ids.clone());

		// Check pending IDs
		let pending = store.peek_pending_ids(conv_id).unwrap();
		assert_eq!(pending.len(), 2);
		assert!(pending.contains(&"tool1".to_string()));
		assert!(pending.contains(&"tool2".to_string()));

		// Fulfill completely
		let result = store.fulfill_complete(conv_id, &tool_ids);
		assert!(result.is_ok());

		// Should be gone now
		assert!(store.peek_pending_ids(conv_id).is_none());
	}

	#[test]
	fn test_tool_cycle_partial_fulfillment() {
		let store = ToolCycleStore::default();
		let conv_id = "test-partial";
		let tool_ids = vec![
			"tool1".to_string(),
			"tool2".to_string(),
			"tool3".to_string(),
		];

		store.insert_cycle(conv_id, tool_ids);

		// Fulfill partially
		let partial_result = store
			.fulfill_partial(conv_id, &["tool1".to_string()])
			.unwrap();
		assert!(!partial_result.complete);
		assert_eq!(partial_result.remaining.len(), 2);

		// Fulfill remaining
		let remaining_result = store
			.fulfill_partial(conv_id, &["tool2".to_string(), "tool3".to_string()])
			.unwrap();
		assert!(remaining_result.complete);
		assert!(remaining_result.remaining.is_empty());
	}

	#[test]
	fn test_unexpected_tool_result() {
		let store = ToolCycleStore::default();
		let conv_id = "test-unexpected";

		store.insert_cycle(conv_id, vec!["tool1".to_string()]);

		// Try to fulfill with unexpected ID
		let result = store.fulfill_complete(conv_id, &["tool2".to_string()]);
		assert!(matches!(result, Err(FulfillmentError::UnexpectedId(_))));
	}

	#[test]
	fn test_missing_tool_results() {
		let store = ToolCycleStore::default();
		let conv_id = "test-missing";

		store.insert_cycle(conv_id, vec!["tool1".to_string(), "tool2".to_string()]);

		// Try to fulfill with only one ID
		let result = store.fulfill_complete(conv_id, &["tool1".to_string()]);
		assert!(matches!(result, Err(FulfillmentError::MissingIds(_))));
	}
}
