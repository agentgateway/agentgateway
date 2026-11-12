use serde::Deserialize;
use serde_json::Value;

use super::{Fields, JsonRef};

#[derive(Clone, Copy)]
pub enum RequestInput<'a> {
	Raw(&'a [u8]),
	Parsed(&'a Value),
}

impl<'a> RequestInput<'a> {
	pub(crate) fn read_raw<T: Deserialize<'a>>(self) -> Option<T> {
		let Self::Raw(bytes) = self else {
			return None;
		};
		serde_json::from_str(std::str::from_utf8(bytes).ok()?).ok()
	}

	pub(crate) fn fields(self) -> Option<Fields<'a>> {
		match self {
			Self::Raw(bytes) => serde_json::from_slice(bytes).ok(),
			Self::Parsed(value) => JsonRef::Parsed(value).fields().ok(),
		}
	}
}

impl<'a> From<&'a [u8]> for RequestInput<'a> {
	fn from(bytes: &'a [u8]) -> Self {
		Self::Raw(bytes)
	}
}
