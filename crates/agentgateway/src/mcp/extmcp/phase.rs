use std::collections::HashMap;

use crate::*;

#[apply(schema_enum!)]
#[derive(Default)]
pub enum Phase {
	#[default]
	Off,
	Request,
	Response,
	Full,
}

impl Phase {
	pub fn runs_request(self) -> bool {
		matches!(self, Phase::Request | Phase::Full)
	}
	pub fn runs_response(self) -> bool {
		matches!(self, Phase::Response | Phase::Full)
	}
}

pub fn resolve(method: &str, methods: &HashMap<String, Phase>) -> Phase {
	if let Some(p) = methods.get(method) {
		return *p;
	}
	methods
		.iter()
		.filter_map(|(pat, phase)| wildcard_literal_len(pat, method).map(|len| (len, *phase)))
		.max_by_key(|(len, _)| *len)
		.map(|(_, phase)| phase)
		.unwrap_or_default()
}

// tiebreaker: if the same method matches multiple patterns, the one with the
// longest literal prefix/suffix wins (~most specific).
fn wildcard_literal_len(pattern: &str, method: &str) -> Option<usize> {
	if pattern == "*" {
		return Some(0);
	}
	if let Some(prefix) = pattern.strip_suffix('*')
		&& !prefix.contains('*')
		&& method.starts_with(prefix)
	{
		return Some(prefix.len());
	}
	if let Some(suffix) = pattern.strip_prefix('*')
		&& !suffix.contains('*')
		&& method.ends_with(suffix)
	{
		return Some(suffix.len());
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	fn methods(pairs: &[(&str, Phase)]) -> HashMap<String, Phase> {
		pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
	}

	#[test]
	fn star_matches_everything() {
		let m = methods(&[("*", Phase::Request)]);
		assert_eq!(resolve("tools/call", &m), Phase::Request);
		assert_eq!(resolve("anything", &m), Phase::Request);
	}

	#[test]
	fn prefix_and_suffix_wildcards() {
		let m = methods(&[("tools/*", Phase::Request), ("*/list", Phase::Response)]);
		assert_eq!(resolve("tools/call", &m), Phase::Request);
		assert_eq!(resolve("prompts/list", &m), Phase::Response);
		assert_eq!(resolve("resources/read", &m), Phase::Off);
	}

	#[test]
	fn most_specific_wins() {
		// exact beats wildcard
		let m = methods(&[("tools/*", Phase::Request), ("tools/call", Phase::Full)]);
		assert_eq!(resolve("tools/call", &m), Phase::Full);
		// longest literal beats shorter / catchall
		let m = methods(&[("*", Phase::Request), ("tools/*", Phase::Full)]);
		assert_eq!(resolve("tools/call", &m), Phase::Full);
	}
}
