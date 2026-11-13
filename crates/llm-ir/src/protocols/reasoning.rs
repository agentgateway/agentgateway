use crate::ir::conversation::Effort;

/// Catalog facts for reasoning effort in Chat Completions and Responses.
/// Missing facts use the renderer's model-name defaults. Reasoning support alone
/// does not imply support for an effort parameter; accepted levels establish that.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReasoningControls {
	/// `Some(false)` suppresses effort, including when levels were also supplied.
	pub supported: Option<bool>,
	efforts: [bool; 6],
}

impl ReasoningControls {
	#[must_use]
	pub fn new(supported: Option<bool>) -> Self {
		Self {
			supported,
			..Self::default()
		}
	}

	#[must_use]
	pub const fn with_effort(mut self, effort: Effort) -> Self {
		self.efforts[effort as usize] = true;
		self
	}

	/// Map to the nearest accepted level, preferring the higher level on a tie,
	/// as Gemini's thinking controls do. An empty set leaves the effort unchanged
	/// when the model-name default accepts it.
	pub(crate) fn effort(self, requested: Effort, default_accepts_effort: bool) -> Option<Effort> {
		if self.supported == Some(false) {
			return None;
		}
		[
			Effort::XHigh,
			Effort::High,
			Effort::Medium,
			Effort::Low,
			Effort::Minimal,
			Effort::Off,
		]
		.into_iter()
		.filter(|effort| self.efforts[*effort as usize])
		.min_by_key(|effort| (*effort as usize).abs_diff(requested as usize))
		.or_else(|| default_accepts_effort.then_some(requested))
	}

	pub(crate) fn accepts(self, effort: Effort) -> bool {
		!self.efforts.contains(&true) || self.efforts[effort as usize]
	}
}
