// Copyright (c) 2024 Max Countryman
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//
// Adapted from headers-accept 0.3.0:
// https://github.com/maxcountryman/headers-accept

use std::cmp::{Ordering, Reverse};
use std::collections::BTreeMap;
use std::str::FromStr;

use headers::{Error as HeaderError, HeaderValue};
use mediatype::{MediaType, MediaTypeBuf, ReadParams, names};

#[derive(Debug)]
pub(super) struct Accept(Vec<MediaTypeBuf>);

impl Accept {
	pub(super) fn decode<'i, I>(values: &mut I) -> Result<Self, HeaderError>
	where
		I: Iterator<Item = &'i HeaderValue>,
	{
		let mut values_iter = values.map(|v| v.to_str().map_err(|_| HeaderError::invalid()));
		let mut value_str = String::from(values_iter.next().ok_or_else(HeaderError::invalid)??);
		for value in values_iter {
			value_str.push(',');
			value_str.push_str(value?);
		}
		Self::parse(&value_str)
	}

	pub(super) fn media_types(&self) -> impl Iterator<Item = &MediaTypeBuf> {
		self.0.iter()
	}

	pub(super) fn negotiate<'a, 'mt: 'a, Available>(
		&self,
		available: Available,
	) -> Option<&'a MediaType<'mt>>
	where
		Available: IntoIterator<Item = &'a MediaType<'mt>>,
	{
		struct BestMediaType<'a, 'mt: 'a> {
			quality: QValue,
			parsed_priority: usize,
			given_priority: usize,
			media_type: &'a MediaType<'mt>,
		}

		available
			.into_iter()
			.enumerate()
			.filter_map(|(given_priority, available_type)| {
				if let Some(matched_range) = self
					.0
					.iter()
					.enumerate()
					.find(|(_, available_range)| MediaRange(available_range) == *available_type)
				{
					let quality = Self::parse_q_value(matched_range.1);
					if quality.is_zero() {
						return None;
					}
					Some(BestMediaType {
						quality,
						parsed_priority: matched_range.0,
						given_priority,
						media_type: available_type,
					})
				} else {
					None
				}
			})
			.max_by_key(|x| (x.quality, Reverse((x.parsed_priority, x.given_priority))))
			.map(|best| best.media_type)
	}

	fn parse(mut s: &str) -> Result<Self, HeaderError> {
		let mut media_types = Vec::new();

		while !s.is_empty() {
			if let Some(index) = s.find(|c: char| !is_ows(c)) {
				s = &s[index..];
			} else {
				break;
			}

			let mut end = 0;
			let mut quoted = false;
			let mut escaped = false;
			for c in s.chars() {
				if escaped {
					escaped = false;
				} else {
					match c {
						'"' => quoted = !quoted,
						'\\' if quoted => escaped = true,
						',' if !quoted => break,
						_ => (),
					}
				}
				end += c.len_utf8();
			}

			match MediaTypeBuf::from_str(s[..end].trim()) {
				Ok(mt) => media_types.push(mt),
				Err(_) => return Err(HeaderError::invalid()),
			}

			s = s[end..].trim_start_matches(',');
		}

		media_types.sort_by_key(|x| {
			let spec = Self::parse_specificity(x);
			let q = Self::parse_q_value(x);
			Reverse((spec, q))
		});

		Ok(Self(media_types))
	}

	fn parse_q_value(media_type: &MediaTypeBuf) -> QValue {
		media_type
			.get_param(names::Q)
			.and_then(|v| v.as_str().parse().ok())
			.unwrap_or_default()
	}

	fn parse_specificity(media_type: &MediaTypeBuf) -> usize {
		let type_specificity = usize::from(media_type.ty() != names::_STAR);
		let subtype_specificity = usize::from(media_type.subty() != names::_STAR);
		let parameter_count = media_type
			.params()
			.filter(|&(name, _)| name != names::Q)
			.count();

		type_specificity + subtype_specificity + parameter_count
	}
}

impl<'a> FromIterator<MediaType<'a>> for Accept {
	fn from_iter<T: IntoIterator<Item = MediaType<'a>>>(iter: T) -> Self {
		Self(iter.into_iter().map(MediaTypeBuf::from).collect())
	}
}

const fn is_ows(c: char) -> bool {
	c == ' ' || c == '\t'
}

struct MediaRange<'a>(&'a MediaTypeBuf);

impl PartialEq<MediaType<'_>> for MediaRange<'_> {
	fn eq(&self, other: &MediaType<'_>) -> bool {
		let (type_match, subtype_match, suffix_match) = (
			self.0.ty() == other.ty,
			self.0.subty() == other.subty,
			self.0.suffix() == other.suffix,
		);

		let wildcard_type = self.0.ty() == names::_STAR;
		let wildcard_subtype = self.0.subty() == names::_STAR && type_match;
		let exact_match = type_match && subtype_match && suffix_match && self.0.params().count() == 0;
		let params_match = type_match && subtype_match && suffix_match && {
			let self_params = self
				.0
				.params()
				.filter(|&(name, _)| name != names::Q)
				.collect::<BTreeMap<_, _>>();
			let other_params = other
				.params()
				.filter(|&(name, _)| name != names::Q)
				.collect::<BTreeMap<_, _>>();

			self_params == other_params
		};

		wildcard_type || wildcard_subtype || exact_match || params_match
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QValue(u16);

impl Default for QValue {
	fn default() -> Self {
		QValue(1000)
	}
}

impl QValue {
	fn is_zero(&self) -> bool {
		self.0 == 0
	}
}

impl FromStr for QValue {
	type Err = HeaderError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		fn parse_fractional(digits: &[u8]) -> Result<u16, HeaderError> {
			digits
				.iter()
				.try_fold(0u16, |acc, &c| {
					if c.is_ascii_digit() {
						Some(acc * 10 + (c - b'0') as u16)
					} else {
						None
					}
				})
				.map(|num| match digits.len() {
					1 => num * 100,
					2 => num * 10,
					_ => num,
				})
				.ok_or_else(HeaderError::invalid)
		}

		match s.as_bytes() {
			b"0" => Ok(QValue(0)),
			b"1" => Ok(QValue(1000)),
			[b'1', b'.', zeros @ ..] if zeros.len() <= 3 && zeros.iter().all(|d| *d == b'0') => {
				Ok(QValue(1000))
			},
			[b'0', b'.', fractional @ ..] if fractional.len() <= 3 => {
				parse_fractional(fractional).map(QValue)
			},
			_ => Err(HeaderError::invalid()),
		}
	}
}

impl Ord for QValue {
	fn cmp(&self, other: &Self) -> Ordering {
		self.0.cmp(&other.0)
	}
}

impl PartialOrd for QValue {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}
