use agent_llm_ir::{ReplyPlace, ReplyRewrite};

pub struct Replace {
	from: &'static str,
	to: &'static str,
	pending: Vec<(ReplyPlace, String)>,
}

impl Replace {
	pub fn new(from: &'static str, to: &'static str) -> Self {
		assert!(!from.is_empty());
		Self {
			from,
			to,
			pending: Vec::new(),
		}
	}
}

impl ReplyRewrite for Replace {
	fn text(&mut self, place: ReplyPlace, text: &str, out: &mut String) {
		let at = self
			.pending
			.iter()
			.position(|(kept_place, _)| *kept_place == place)
			.unwrap_or_else(|| {
				self.pending.push((place, String::new()));
				self.pending.len() - 1
			});
		let pending = &mut self.pending[at].1;
		pending.push_str(text);
		let mut start = 0;
		for (at, _) in pending.match_indices(self.from) {
			out.push_str(&pending[start..at]);
			out.push_str(self.to);
			start = at + self.from.len();
		}
		let tail = &pending[start..];
		// Keep only a suffix that could become a match in the next delta.
		let keep = tail
			.char_indices()
			.find(|(at, _)| self.from.starts_with(&tail[*at..]))
			.map_or(tail.len(), |(at, _)| at);
		out.push_str(&tail[..keep]);
		pending.drain(..start + keep);
	}

	fn finish(&mut self, place: ReplyPlace, out: &mut String) {
		if let Some(at) = self
			.pending
			.iter()
			.position(|(kept_place, _)| *kept_place == place)
		{
			out.push_str(&self.pending.remove(at).1);
		}
	}
}
