//! How downstream-visible tool/prompt names and resource URIs map to upstream
//! targets. `NameRouting` owns every synchronous decision the mode implies:
//! outbound encoding, inbound decoding, and the uniqueness contract.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use agent_core::prelude::Strng;
use itertools::Itertools;
use tracing::warn;

use crate::mcp::apps;
use crate::mcp::router::McpTarget;
use crate::mcp::upstream::UpstreamError;
use crate::types::agent::McpPrefixMode;

const DELIMITER: &str = "_";

/// How downstream-visible tool/prompt names map to upstream targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameRouting {
	/// Single target: names pass through untouched and everything routes to it.
	Single(String),
	/// Names are exposed as `{target}_{name}`; the prefix is parsed off to route.
	Prefix,
	/// Names pass through untouched; calls are routed by looking up which
	/// target serves the name. Requires names to be unique across targets.
	Resolve,
}

/// Result of decoding a downstream name: either the name itself carried the
/// route, or the owning target must be resolved against upstream listings.
pub enum NameDecision<'a, 'b> {
	Target { service: &'a str, name: &'b str },
	NeedsResolution,
}

impl NameRouting {
	pub fn new(prefix_mode: McpPrefixMode, targets: &[Arc<McpTarget>]) -> Self {
		if targets.len() != 1 {
			match prefix_mode {
				McpPrefixMode::Never => NameRouting::Resolve,
				_ => NameRouting::Prefix,
			}
		} else if prefix_mode == McpPrefixMode::Always {
			NameRouting::Prefix
		} else {
			NameRouting::Single(targets[0].name.to_string())
		}
	}

	/// Whether the owning target must be resolved against upstream listings.
	pub fn needs_resolution(&self) -> bool {
		matches!(self, NameRouting::Resolve)
	}

	/// Whether resource URIs carry the target name (`{target}+{scheme}://...`).
	/// Unlike names, URIs stay encoded in Resolve mode: clients only ever see
	/// URIs we produced, so the encoding is transparent to them, and it is how
	/// we route resource reads and Apps ui:// resources back to their target.
	pub fn encodes_uris(&self) -> bool {
		!matches!(self, NameRouting::Single(_))
	}

	/// Downstream-visible form of an upstream tool/prompt name.
	pub fn encode_name(&self, target: &str, name: &str) -> String {
		match self {
			NameRouting::Prefix => format!("{target}{DELIMITER}{name}"),
			_ => name.to_string(),
		}
	}

	/// Downstream-visible form of an upstream resource URI.
	pub fn encode_uri(&self, target: &str, uri: &str) -> String {
		if !self.encodes_uris() {
			return uri.to_string();
		}
		// Apps UI resources must keep their ui:// scheme so hosts still
		// recognize them; the target is carried in the authority instead.
		if let Some(rewritten) = apps::encode_ui_uri(target, uri) {
			return rewritten;
		}
		// Transform URI to service+scheme:// format for multiplexing
		// e.g., "http://example.com" becomes "service+http://example.com"
		if let Some(scheme_end) = uri.find("://") {
			let (scheme, rest) = uri.split_at(scheme_end);
			format!("{target}+{scheme}{rest}")
		} else {
			// URI must have a scheme - if not, return as-is and let validation handle it
			uri.to_string()
		}
	}

	/// Reverse of `encode_name`: map a downstream name back to its route.
	pub fn decode_name<'a, 'b: 'a>(
		&'a self,
		res: &'b str,
	) -> Result<NameDecision<'a, 'b>, UpstreamError> {
		match self {
			NameRouting::Single(default) => Ok(NameDecision::Target {
				service: default.as_str(),
				name: res,
			}),
			NameRouting::Prefix => res
				.split_once(DELIMITER)
				.map(|(service, name)| NameDecision::Target { service, name })
				.ok_or(UpstreamError::InvalidRequest(
					"invalid resource name".to_string(),
				)),
			NameRouting::Resolve => Ok(NameDecision::NeedsResolution),
		}
	}

	/// Reverse of `encode_uri`: extract the (unvalidated) service name and the
	/// original URI from a downstream URI. The caller is responsible for
	/// validating the service name against the known targets.
	pub fn decode_uri<'a, 'b: 'a>(
		&'a self,
		uri: &'b str,
	) -> Result<(&'a str, String), UpstreamError> {
		if let NameRouting::Single(default) = self {
			return Ok((default.as_str(), uri.to_string()));
		}
		if apps::is_ui_uri(uri) {
			return apps::decode_ui_uri(uri)
				.ok_or_else(|| UpstreamError::InvalidRequest("invalid resource URI".to_string()));
		}
		// URI format: "service+scheme://rest"
		let (service, original_uri) = uri
			.split_once('+')
			.ok_or_else(|| UpstreamError::InvalidRequest("invalid resource URI".to_string()))?;
		// ui:// resources use the ui://service+rest namespace exclusively
		if apps::is_ui_uri(original_uri) {
			return Err(UpstreamError::InvalidRequest(
				"invalid resource URI".to_string(),
			));
		}
		Ok((service, original_uri.to_string()))
	}

	/// Names served by more than one target, which are unroutable in Resolve
	/// mode and dropped from merged listings; empty for the other modes.
	pub fn duplicate_names<'a>(
		&self,
		kind: &str,
		names: impl Iterator<Item = (&'a Strng, &'a str)>,
	) -> HashSet<String> {
		if !self.needs_resolution() {
			return HashSet::new();
		}
		let mut owners: HashMap<&str, Vec<&Strng>> = HashMap::new();
		for (target, name) in names {
			owners.entry(name).or_default().push(target);
		}
		owners
			.into_iter()
			.filter(|(_, targets)| targets.len() > 1)
			.map(|(name, targets)| {
				warn!(
					"dropping {kind} '{name}': served by multiple targets ({})",
					targets.iter().join(", "),
				);
				name.to_string()
			})
			.collect()
	}
}
