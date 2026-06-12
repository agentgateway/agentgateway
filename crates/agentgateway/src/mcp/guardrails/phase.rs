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

// Match precedence, most specific first:
//   1. exact match (`tools/call`)
//   2. prefix wildcard (`tools/*`) beats suffix wildcard (`*/list`) -- method
//      names are namespaced left-to-right, so the namespace owner wins
//   3. suffix wildcard beats the `*` catchall
//   4. within the same kind, the longer pattern wins (`notifications/tools/*`
//      over `notifications/*`); the trailing `*` is constant within a kind so
//      it cancels out
//   5. ties broken alphabetically so resolution is deterministic
pub fn resolve(method: &str, methods: &HashMap<String, Phase>) -> Phase {
	if let Some(p) = methods.get(method) {
		return *p;
	}
	methods
		.iter()
		.filter_map(|(pat, phase)| match_kind(pat, method).map(|kind| (kind, pat, *phase)))
		// (kind, len) ascending; alphabetically-first pattern wins the final tie
		.max_by(|a, b| {
			(a.0, a.1.len())
				.cmp(&(b.0, b.1.len()))
				.then_with(|| b.1.cmp(a.1))
		})
		.map(|(_, _, phase)| phase)
		.unwrap_or_default()
}

/// Whether a methods key is a pattern `resolve` can ever match: a non-empty
/// exact name, `*`, or a single leading or trailing `*`.
pub fn pattern_is_matchable(pat: &str) -> bool {
	if pat.is_empty() {
		return false;
	}
	match pat.matches('*').count() {
		0 => true,
		1 => pat.starts_with('*') || pat.ends_with('*'),
		_ => false,
	}
}

// Higher rank = more specific. None if the pattern doesn't match.
fn match_kind(pattern: &str, method: &str) -> Option<u8> {
	if pattern == "*" {
		return Some(1);
	}
	if let Some(prefix) = pattern.strip_suffix('*')
		&& !prefix.contains('*')
		&& method.starts_with(prefix)
	{
		return Some(3);
	}
	if let Some(suffix) = pattern.strip_prefix('*')
		&& !suffix.contains('*')
		&& method.ends_with(suffix)
	{
		return Some(2);
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
	fn exact_beats_wildcard() {
		let m = methods(&[("tools/*", Phase::Request), ("tools/call", Phase::Full)]);
		assert_eq!(resolve("tools/call", &m), Phase::Full);
		assert_eq!(resolve("tools/list", &m), Phase::Request);
	}

	#[test]
	fn prefix_beats_suffix() {
		// tools/list matches both; prefix (namespace owner) wins
		let m = methods(&[("tools/*", Phase::Request), ("*/list", Phase::Response)]);
		assert_eq!(resolve("tools/list", &m), Phase::Request);
		// prefix wins even though the suffix literal `/setLevel` is longer than
		// the prefix literal `logging/`
		let m = methods(&[
			("logging/*", Phase::Request),
			("*/setLevel", Phase::Response),
		]);
		assert_eq!(resolve("logging/setLevel", &m), Phase::Request);
	}

	#[test]
	fn wildcards_beat_catchall() {
		// suffix beats `*`
		let m = methods(&[("*", Phase::Request), ("*/list", Phase::Response)]);
		assert_eq!(resolve("resources/list", &m), Phase::Response);
		// prefix beats `*`
		let m = methods(&[("*", Phase::Request), ("tools/*", Phase::Full)]);
		assert_eq!(resolve("tools/call", &m), Phase::Full);
	}

	#[test]
	fn longer_prefix_wins() {
		// resources/templates/list matches both prefixes; the more specific
		// (longer) namespace wins
		let m = methods(&[
			("resources/*", Phase::Request),
			("resources/templates/*", Phase::Response),
		]);
		assert_eq!(resolve("resources/templates/list", &m), Phase::Response);
		assert_eq!(resolve("resources/read", &m), Phase::Request);
	}
}
