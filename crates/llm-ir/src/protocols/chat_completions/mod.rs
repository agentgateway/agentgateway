#![doc = include_str!("README.md")]

//! Chat Completions as a client and as a target: OpenAI and the engines that copy its format.
//!
//! Clients return tool-call IDs, so provider reasoning before a call is carried
//! in an appended replay carrier and restored before that call on the next turn.
//! Reasoning after the last call, or in a reply without calls, is not replayed.

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::carrier::{self, CarrierError, StateScope};
use crate::json::push_string;

pub mod decoder;
pub(crate) mod parse;
pub mod render;
pub mod writer;

/// Separates the provider's call ID from its carrier. Both use `[A-Za-z0-9_-]`
/// because some providers and proxies reject other characters.
/// The version is encoded in the carrier, so this marker need not change with it.
const CARRIER_MARKER: &str = "__agentgateway_llm_ir_";

pub(crate) struct CarriedReasoning {
	pub text_offset: usize,
	pub item: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReplayedReasoning {
	pub text_offset: usize,
	pub item: Vec<u8>,
}

/// `items` are the provider's reasoning items and their positions in visible answer text.
pub(crate) fn encode_call_id(
	scope: StateScope<'_>,
	call: &str,
	items: &[CarriedReasoning],
) -> String {
	if items.is_empty() {
		return call.to_string();
	}
	let mut state = String::from(r#"{"call":"#);
	push_string(&mut state, call);
	state.push_str(r#","items":["#);
	for (at, item) in items.iter().enumerate() {
		if at > 0 {
			state.push(',');
		}
		state.push_str(r#"{"text_offset":"#);
		state.push_str(&item.text_offset.to_string());
		state.push_str(r#", "item":"#);
		state.push_str(&item.item);
		state.push('}');
	}
	state.push_str("]}");
	format!(
		"{call}{CARRIER_MARKER}{}",
		carrier::encode_unprefixed(scope, carrier::CarrierPurpose::ChatCallId, state.as_bytes(),)
	)
}

/// The provider's call id, and the carrier the gateway added to it, if any.
pub(crate) fn split_call_id(id: &str) -> (&str, Option<&str>) {
	match id.rfind(CARRIER_MARKER) {
		Some(at) => (&id[..at], Some(&id[at + CARRIER_MARKER.len()..])),
		None => (id, None),
	}
}

/// The reasoning items encoded for `call`, each the state of a carrier of its own.
pub(crate) fn open_call_id(
	scope: StateScope<'_>,
	call: &str,
	carrier: &str,
) -> Result<Vec<ReplayedReasoning>, CarrierError> {
	#[derive(Deserialize)]
	struct Item<'a> {
		text_offset: usize,
		#[serde(borrow)]
		item: &'a RawValue,
	}

	#[derive(Deserialize)]
	struct State<'a> {
		#[serde(borrow)]
		call: Cow<'a, str>,
		#[serde(borrow)]
		items: Vec<Item<'a>>,
	}
	let state = carrier::open_unprefixed(scope, carrier::CarrierPurpose::ChatCallId, carrier)?;
	let state: State<'_> = serde_json::from_slice(&state).map_err(|_| CarrierError::Malformed)?;
	// Encoded for another call: moved there by the client.
	if state.call != call {
		return Err(CarrierError::Malformed);
	}
	Ok(
		state
			.items
			.iter()
			.map(|item| ReplayedReasoning {
				text_offset: item.text_offset,
				item: item.item.get().as_bytes().to_vec(),
			})
			.collect(),
	)
}

#[cfg(test)]
mod tests {
	#![allow(clippy::unwrap_used)]

	use super::*;
	use crate::test_support::SCOPE;

	const ITEMS: [&str; 2] = [r#"{"type":"thinking","signature":"a"}"#, r#"{"b":[1]}"#];

	fn encoded() -> String {
		let items: Vec<CarriedReasoning> = ITEMS
			.iter()
			.map(|item| CarriedReasoning {
				text_offset: 0,
				item: item.to_string(),
			})
			.collect();
		encode_call_id(SCOPE, "call_1", &items)
	}

	#[test]
	fn items_encoded_into_an_id_open_for_that_call() {
		let id = encoded();
		let (call, carrier) = split_call_id(&id);
		assert_eq!(call, "call_1");
		let items = open_call_id(SCOPE, call, carrier.unwrap()).unwrap();
		let items: Vec<&str> = items
			.iter()
			.map(|item| std::str::from_utf8(&item.item).unwrap())
			.collect();
		assert_eq!(items, ITEMS);
	}

	/// Some providers and proxies take only these characters in a call id.
	#[test]
	fn a_encoded_id_is_url_safe() {
		let id = encoded();
		assert!(
			id.bytes()
				.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
			"{id}"
		);
	}

	#[test]
	fn an_id_without_a_carrier_is_the_providers() {
		assert_eq!(split_call_id("call~1"), ("call~1", None));
		assert_eq!(split_call_id("call_1"), ("call_1", None));
	}

	#[test]
	fn a_carrier_moved_to_another_call_or_scope_doesnt_open() {
		let id = encoded();
		let (_, carrier) = split_call_id(&id);
		let carrier = carrier.unwrap();
		assert_eq!(
			open_call_id(SCOPE, "call_2", carrier),
			Err(CarrierError::Malformed)
		);
		let other = StateScope {
			model: "other",
			..SCOPE
		};
		assert_eq!(
			open_call_id(other, "call_1", carrier),
			Err(CarrierError::WrongScope)
		);
	}
}
