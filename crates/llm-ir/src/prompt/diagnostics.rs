//! Every problem is kept, so one reply lists them all.

use std::borrow::Cow;
use std::fmt;
use std::num::NonZeroU64;

use super::RequestRefusal;
use crate::ir::conversation::ImageFields;
use crate::json::{Fields, JsonRef, text_of};

#[derive(Default)]
pub struct Diagnostics {
	pub(crate) problems: Vec<String>,
	pub(crate) dropped: Vec<&'static str>,
}

impl Diagnostics {
	pub(crate) fn finish(self) -> Result<Vec<&'static str>, RequestRefusal> {
		if self.problems.is_empty() {
			Ok(self.dropped)
		} else {
			Err(RequestRefusal {
				problems: self.problems,
			})
		}
	}

	pub(crate) fn list<'a>(
		&mut self,
		path: impl fmt::Display,
		raw: impl Into<JsonRef<'a>>,
	) -> Option<Vec<JsonRef<'a>>> {
		match raw.into().list() {
			Ok(list) => Some(list),
			Err(_) => {
				self.problem(format!("`{path}` must be a list"));
				None
			},
		}
	}

	pub(crate) fn object<'a>(
		&mut self,
		path: impl fmt::Display,
		raw: impl Into<JsonRef<'a>>,
	) -> Option<Fields<'a>> {
		let Ok(fields) = raw.into().fields() else {
			self.problem(format!("`{path}` must be an object"));
			return None;
		};
		(!self.reject_duplicates(path, &fields)).then_some(fields)
	}

	/// `path` is written out only if a field repeats.
	pub(crate) fn reject_duplicates(&mut self, path: impl fmt::Display, fields: &Fields<'_>) -> bool {
		let mut names: Vec<&str> = fields.0.iter().map(|(name, _)| name.as_ref()).collect();
		names.sort_unstable();
		let repeated = names.windows(2).find(|pair| pair[0] == pair[1]);
		if let Some(pair) = repeated {
			self.problem(format!("`{path}` repeats `{}`", pair[0]));
		}
		repeated.is_some()
	}

	pub(crate) fn typed<'a, T: serde::de::DeserializeOwned>(
		&mut self,
		path: impl fmt::Display,
		raw: impl Into<JsonRef<'a>>,
	) -> Option<T> {
		match raw.into().read() {
			Ok(value) => Some(value),
			Err(_) => {
				self.problem(format!("`{path}` has the wrong type"));
				None
			},
		}
	}

	pub(crate) fn text<'a>(
		&mut self,
		path: impl fmt::Display,
		raw: impl Into<JsonRef<'a>>,
	) -> Option<Cow<'a, str>> {
		let text = text_of(raw);
		if text.is_none() {
			self.problem(format!("`{path}` has the wrong type"));
		}
		text
	}

	pub(crate) fn token_limit<'a>(
		&mut self,
		path: impl fmt::Display,
		raw: impl Into<JsonRef<'a>>,
	) -> TokenLimit {
		let Some(limit) = self.typed::<u64>(&path, raw) else {
			return TokenLimit::Unreadable;
		};
		match NonZeroU64::new(limit) {
			Some(limit) => TokenLimit::Positive(limit),
			None => {
				self.problem(format!("`{path}` must be positive"));
				TokenLimit::Zero
			},
		}
	}

	pub(crate) fn problem(&mut self, problem: impl Into<String>) {
		self.problems.push(problem.into());
	}

	pub(crate) fn drop_field(&mut self, field: &'static str) {
		if !self.dropped.contains(&field) {
			self.dropped.push(field);
		}
	}
}

/// Parsed output limit, keeping zero distinct so it cannot use a default.
pub enum TokenLimit {
	Unreadable,
	Zero,
	Positive(NonZeroU64),
}

/// Source content that couldn't enter the conversation. Target checks use it only
/// to finish reporting problems when parsing has already refused the request.
#[derive(Default)]
pub struct Unread<'a> {
	/// In message order.
	pub(crate) output_configs: Vec<UnreadConfig<'a>>,
	/// User messages with a text block that was refused. They still started a turn.
	pub(crate) refused_text: Vec<usize>,
	pub(crate) images: Vec<UnreadImage<'a>>,
}

pub struct UnreadImage<'a> {
	pub(crate) path: String,
	pub(crate) fields: ImageFields<'a>,
}

impl Unread<'_> {
	pub(crate) const fn is_empty(&self) -> bool {
		self.output_configs.is_empty() && self.refused_text.is_empty() && self.images.is_empty()
	}
}

pub struct UnreadConfig<'a> {
	pub(crate) index: usize,
	pub(crate) raw: JsonRef<'a>,
	/// The message had a role and content, so it counts as a turn's setting.
	pub(crate) in_turn: bool,
}
