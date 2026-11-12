//! Fakes shared by unit tests.

use crate::carrier::{IdentifierPseudonymizer, StateScope};
use crate::prompt::{ConvertedRequest, RenderSettings, RequestRefusal, Upstream, prepare};
use crate::protocols::ClientEndpoint;

pub const SCOPE: StateScope<'static> = StateScope {
	backend: "a",
	provider: "p",
	model: "m",
};

/// A small deterministic hash for identifier tests.
pub struct Sum;

impl IdentifierPseudonymizer for Sum {
	fn pseudonymize(&self, identifier: &str) -> String {
		let mut hash = 0xcbf2_9ce4_8422_2325u64;
		for byte in identifier.bytes() {
			hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
		}
		format!("{hash:016x}")
	}
}

pub fn text_chunks<'a>(text: &'a str, cuts: &[usize]) -> Vec<&'a str> {
	let boundaries: Vec<_> = text
		.char_indices()
		.map(|(at, _)| at)
		.chain([text.len()])
		.collect();
	let mut offsets: Vec<_> = cuts
		.iter()
		.map(|cut| boundaries[cut % boundaries.len()])
		.collect();
	offsets.extend([0, text.len()]);
	offsets.sort_unstable();
	offsets.dedup();
	offsets
		.windows(2)
		.map(|pair| &text[pair[0]..pair[1]])
		.collect()
}

/// Converts a request with model-derived render settings and no edits.
pub fn convert(
	body: &[u8],
	client: impl Into<ClientEndpoint>,
	upstream: Upstream<'_>,
) -> Result<ConvertedRequest, RequestRefusal> {
	prepare(body, client, upstream)?.render(RenderSettings::ModelDefaults)
}
