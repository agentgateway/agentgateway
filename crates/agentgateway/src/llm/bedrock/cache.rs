//! Prompt caching for Anthropic Messages API → Bedrock translation

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::llm::bedrock::types::{CachePointBlock, CachePointType, SystemContentBlock};

/// Configuration for prompt cache planning
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachePlannerConfig {
	/// Minimum tokens required before inserting cache point (default: 1024)
	pub min_tokens: usize,

	/// Safety margin added to min_tokens to ensure cache effectiveness (default: 76)
	pub safety_margin: usize,

	/// Whether caching is enabled
	pub enabled: bool,

	/// Force cache point insertion regardless of threshold (for experimentation)
	pub force: bool,
}

/// Result of cache planning operation
#[derive(Debug, Clone)]
pub struct CachePlan {
	/// Whether a cache point was inserted
	pub inserted: bool,

	/// Estimated tokens before cache point
	pub estimated_tokens: usize,

	/// Reason for decision (for debugging)
	pub reason: String,

	/// Position where cache point was inserted (if any)
	pub insertion_position: Option<usize>,
}

impl Default for CachePlannerConfig {
	fn default() -> Self {
		Self {
			min_tokens: 1024,
			safety_margin: 76,
			enabled: true,
			force: false,
		}
	}
}

/// Plan and insert cache point in system content blocks
pub fn plan_and_insert_cachepoint(
	system_blocks: &mut Vec<SystemContentBlock>,
	tool_estimated_weight: usize,
	config: &CachePlannerConfig,
) -> CachePlan {
	if !config.enabled && !config.force {
		return CachePlan {
			inserted: false,
			estimated_tokens: 0,
			reason: "Caching disabled".to_string(),
			insertion_position: None,
		};
	}

	// Calculate total estimated tokens including tools
	let system_tokens = estimate_system_tokens(system_blocks);
	let total_tokens = system_tokens + tool_estimated_weight;
	let threshold = config.min_tokens + config.safety_margin;

	debug!(
		"Cache planning: system_tokens={}, tool_weight={}, total={}, threshold={}",
		system_tokens, tool_estimated_weight, total_tokens, threshold
	);

	// Decide whether to insert cache point
	let should_cache = config.force || total_tokens >= threshold;

	if !should_cache {
		return CachePlan {
			inserted: false,
			estimated_tokens: total_tokens,
			reason: format!("Below threshold: {} < {}", total_tokens, threshold),
			insertion_position: None,
		};
	}

	// Find strategic insertion position
	let insertion_pos = find_optimal_cache_position(system_blocks);

	if let Some(pos) = insertion_pos {
		// Insert cache point at optimal position
		insert_cache_point_at_position(system_blocks, pos);

		CachePlan {
			inserted: true,
			estimated_tokens: total_tokens,
			reason: format!("Cache point inserted at position {}", pos),
			insertion_position: Some(pos),
		}
	} else {
		CachePlan {
			inserted: false,
			estimated_tokens: total_tokens,
			reason: "No suitable insertion position found".to_string(),
			insertion_position: None,
		}
	}
}

/// Estimate token count for system content blocks
fn estimate_system_tokens(system_blocks: &[SystemContentBlock]) -> usize {
	system_blocks
		.iter()
		.map(|block| match block {
			SystemContentBlock::Text(text) => {
				// Rough estimate: ~4 characters per token for English text
				(text.len() as f64 / 4.0).ceil() as usize
			},
			SystemContentBlock::CachePoint(_) => 0, // Cache points don't add tokens
		})
		.sum()
}

/// Find optimal position for cache point insertion
fn find_optimal_cache_position(system_blocks: &[SystemContentBlock]) -> Option<usize> {
	if system_blocks.is_empty() {
		return None;
	}

	// Strategy: Insert cache point before the last text block to maximize
	// the amount of cached content while allowing dynamic content to remain uncached

	// Find the last text block
	for (i, block) in system_blocks.iter().enumerate().rev() {
		if matches!(block, SystemContentBlock::Text(_)) {
			// Insert before this text block if it's not the first
			if i > 0 {
				return Some(i);
			}
			// If it's the first block, insert after it
			return Some(i + 1);
		}
	}

	// If no text blocks found, insert at the end
	Some(system_blocks.len())
}

/// Insert cache point at the specified position
fn insert_cache_point_at_position(system_blocks: &mut Vec<SystemContentBlock>, position: usize) {
	let cache_point = SystemContentBlock::CachePoint(CachePointBlock {
		cache_type: CachePointType::Default,
	});

	if position <= system_blocks.len() {
		system_blocks.insert(position, cache_point);
		debug!("Inserted cache point at position {}", position);
	} else {
		warn!(
			"Invalid cache point position {}, appending to end",
			position
		);
		system_blocks.push(cache_point);
	}
}

/// Estimate tool schema weight for cache planning
pub fn estimate_tool_schema_weight(tools: &[crate::llm::anthropic_types::Tool]) -> usize {
	tools
		.iter()
		.map(|tool| {
			// Estimate tokens for tool name, description, and schema
			let name_tokens = (tool.name.len() as f64 / 4.0).ceil() as usize;
			let desc_tokens = tool
				.description
				.as_ref()
				.map(|desc| (desc.len() as f64 / 4.0).ceil() as usize)
				.unwrap_or(0);

			// JSON schema is more token-dense, estimate ~3 chars per token
			let schema_tokens = if let Ok(schema_str) = serde_json::to_string(&tool.input_schema) {
				(schema_str.len() as f64 / 3.0).ceil() as usize
			} else {
				50 // Fallback estimate
			};

			name_tokens + desc_tokens + schema_tokens
		})
		.sum()
}

/// Enhanced configuration for production use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancedCacheConfig {
	/// Base configuration
	#[serde(flatten)]
	pub base: CachePlannerConfig,

	/// Whether to include tool schema weight in token estimation
	#[serde(default = "default_true")]
	pub include_tool_weight: bool,

	/// Maximum number of cache points to insert per request
	#[serde(default = "default_max_cache_points")]
	pub max_cache_points: usize,

	/// Minimum gap between cache points (in estimated tokens)
	#[serde(default = "default_min_gap")]
	pub min_cache_point_gap: usize,
}

fn default_true() -> bool {
	true
}
fn default_max_cache_points() -> usize {
	1
}
fn default_min_gap() -> usize {
	512
}

impl Default for EnhancedCacheConfig {
	fn default() -> Self {
		Self {
			base: CachePlannerConfig::default(),
			include_tool_weight: true,
			max_cache_points: 1,
			min_cache_point_gap: 512,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_cache_planning_below_threshold() {
		let mut system_blocks = vec![SystemContentBlock::Text("Short system prompt".to_string())];

		let config = CachePlannerConfig {
			min_tokens: 1000,
			safety_margin: 100,
			enabled: true,
			force: false,
		};

		let plan = plan_and_insert_cachepoint(&mut system_blocks, 0, &config);

		assert!(!plan.inserted);
		assert!(plan.reason.contains("Below threshold"));
		assert_eq!(system_blocks.len(), 1); // No cache point inserted
	}

	#[test]
	fn test_cache_planning_above_threshold() {
		let long_text = "Lorem ipsum ".repeat(200); // ~2400 characters, ~600 tokens
		let mut system_blocks = vec![SystemContentBlock::Text(long_text)];

		let config = CachePlannerConfig {
			min_tokens: 400,
			safety_margin: 50,
			enabled: true,
			force: false,
		};

		let plan = plan_and_insert_cachepoint(&mut system_blocks, 100, &config);

		assert!(plan.inserted);
		assert!(plan.estimated_tokens > 450);
		assert_eq!(system_blocks.len(), 2); // Cache point inserted
	}

	#[test]
	fn test_force_cache_insertion() {
		let mut system_blocks = vec![SystemContentBlock::Text("Short".to_string())];

		let config = CachePlannerConfig {
			min_tokens: 10000,
			safety_margin: 1000,
			enabled: true,
			force: true, // Force insertion regardless of threshold
		};

		let plan = plan_and_insert_cachepoint(&mut system_blocks, 0, &config);

		assert!(plan.inserted);
		assert_eq!(system_blocks.len(), 2); // Cache point inserted despite low tokens
	}

	#[test]
	fn test_tool_schema_weight_estimation() {
		let tools = vec![crate::llm::anthropic_types::Tool {
			name: "get_weather".to_string(),
			description: Some("Get current weather for a location".to_string()),
			input_schema: serde_json::json!({
					"type": "object",
					"properties": {
							"location": {
									"type": "string",
									"description": "City name"
							}
					},
					"required": ["location"]
			}),
			cache_control: None,
		}];

		let weight = estimate_tool_schema_weight(&tools);
		assert!(weight > 0);
		assert!(weight > 20); // Should be reasonable estimate
	}
}
