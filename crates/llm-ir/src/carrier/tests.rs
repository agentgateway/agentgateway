#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use rstest::rstest;

use super::*;

const SCOPE: StateScope<'static> = StateScope {
	backend: "a",
	provider: "openai",
	model: "m",
};
const PURPOSE: CarrierPurpose = CarrierPurpose::MessagesSignature;

proptest! {
	#![proptest_config(ProptestConfig::with_cases(128))]

	#[test]
	fn replay_round_trips_in_each_client_field(state in prop::collection::vec(any::<u8>(), 0..512)) {
		for purpose in [CarrierPurpose::MessagesSignature, CarrierPurpose::ResponsesEncryptedContent,
			CarrierPurpose::ChatCallId, CarrierPurpose::GeminiThoughtSignature] {
			let carrier = encode(SCOPE, purpose, &state);
			prop_assert_eq!(open(SCOPE, purpose, &carrier), Ok(state.clone()));
			let carrier = encode_unprefixed(SCOPE, purpose, &state);
			prop_assert_eq!(open_unprefixed(SCOPE, purpose, &carrier), Ok(state.clone()));
		}
	}

	#[test]
	fn replay_refuses_a_different_scope_or_client_field(
		backend in ".{0,24}", provider in ".{0,24}", model in ".{0,24}",
		state in prop::collection::vec(any::<u8>(), 0..128), purpose in 0..4usize,
	) {
		let purposes = [CarrierPurpose::MessagesSignature, CarrierPurpose::ResponsesEncryptedContent,
			CarrierPurpose::ChatCallId, CarrierPurpose::GeminiThoughtSignature];
		let other = purposes[(purpose + 1) % purposes.len()];
		let purpose = purposes[purpose];
		let scope = StateScope { backend: &backend, provider: &provider, model: &model };
		let carrier = encode(scope, purpose, &state);
		let changed = format!("{backend}x");
		prop_assert_eq!(open(StateScope {backend: &changed, ..scope}, purpose, &carrier), Err(CarrierError::WrongScope));
		let changed = format!("{provider}x");
		prop_assert_eq!(open(StateScope {provider: &changed, ..scope}, purpose, &carrier), Err(CarrierError::WrongScope));
		let changed = format!("{model}x");
		prop_assert_eq!(open(StateScope {model: &changed, ..scope}, purpose, &carrier), Err(CarrierError::WrongScope));
		prop_assert_eq!(open(scope, other, &carrier), Err(CarrierError::WrongScope));
	}

	#[test]
	fn scope_fields_cannot_shift_into_each_other(prefix in ".{0,24}", middle in ".{1,24}", suffix in ".{0,24}") {
		let backend = format!("{prefix}{middle}");
		let provider = format!("{middle}{suffix}");
		let first = StateScope { backend: &backend, provider: &suffix, model: "m" };
		let second = StateScope { backend: &prefix, provider: &provider, model: "m" };
		let carrier = encode(first, PURPOSE, b"state");
		prop_assert_eq!(open(second, PURPOSE, &carrier), Err(CarrierError::WrongScope));
	}

	#[test]
	fn arbitrary_carrier_text_does_not_panic(text in ".{0,512}") {
		let _ = open(SCOPE, PURPOSE, &text);
		let _ = open_unprefixed(SCOPE, PURPOSE, &text);
	}

	#[test]
	fn client_can_read_and_edit_the_carried_state(state in prop::collection::vec(any::<u8>(), 1..256)) {
		let carrier = encode(SCOPE, PURPOSE, &state);
		let mut bytes = decode(carrier.strip_prefix(PREFIX).unwrap()).unwrap();
		prop_assert!(bytes.ends_with(&state));
		let last = bytes.last_mut().unwrap();
		*last ^= 1;
		let edited = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes));
		let mut expected = state;
		*expected.last_mut().unwrap() ^= 1;
		prop_assert_eq!(open(SCOPE, PURPOSE, &edited), Ok(expected));
	}


}

#[rstest]
fn other_formats_are_refused(
	#[values(
		"agentgateway.responses.v1:abc",
		"agentgateway.responses.v2:abc",
		"agentgateway.llm-ir.v2:abc.def",
		"agentgateway.llm-ir.v3:abc",
		"",
		"abc"
	)]
	carrier: &str,
) {
	assert_eq!(
		open(SCOPE, PURPOSE, carrier),
		Err(CarrierError::UnknownFormat)
	);
}

#[rstest]
#[case(b"", "")]
#[case(b"\xfb", "-w")]
#[case(b"\xfb\xff", "-_8")]
#[case(b"\xfb\xff\xfe", "-__-")]
#[case(b"\x00\x01\xfc\xff", "AAH8_w")]
fn base64_has_one_spelling(#[case] bytes: &[u8], #[case] text: &str) {
	let mut out = String::new();
	URL_SAFE_NO_PAD.encode_string(bytes, &mut out);
	assert_eq!(out, text);
	assert_eq!(decode(text).unwrap(), bytes);
}

#[rstest]
#[case::spare_bits("-x")]
#[case::spare_bits_after_two_bytes("-_9")]
#[case::lone_letter("A")]
#[case::lone_letter_after_a_group("AAAAA")]
#[case::padding("-w==")]
#[case::standard_plus("+w")]
#[case::standard_slash("/w")]
fn other_base64_spellings_are_malformed(#[case] text: &str) {
	assert_eq!(decode(text), Err(CarrierError::Malformed));
}
