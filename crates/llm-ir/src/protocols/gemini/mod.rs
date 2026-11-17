#![doc = include_str!("README.md")]

//! Google Gemini, through `generateContent`.
//!
//! A Gemini client keeps replay state in a part's `thoughtSignature`, which Google's SDKs
//! treat as base64 bytes. One signature contains every carrier that came before its part,
//! separated by spaces, in standard base64. A carrier is ASCII from the URL-safe
//! alphabet, so its base64 needs neither `+` nor `/`; a client that re-encodes the
//! bytes may still change the alphabet or drop the padding, so either is read.

use base64::engine::general_purpose::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::{Engine, alphabet};

use crate::carrier::PREFIX;

pub mod decoder;
pub(crate) mod parse;
pub mod render;
mod schema;
pub mod writer;

/// What Google documents a client can send where it has no signature of its own.
const PLACEHOLDERS: [&str; 2] = [
	"skip_thought_signature_validator",
	"context_engineering_is_the_way_to_go",
];

const LENIENT: GeneralPurpose = GeneralPurpose::new(
	&alphabet::STANDARD,
	GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

pub(crate) fn write_signature(carriers: &[String]) -> String {
	assert!(
		!carriers.is_empty(),
		"a signature holds at least one carrier"
	);
	STANDARD.encode(carriers.join(" "))
}

pub(crate) enum Signature {
	Carriers(Vec<String>),
	Placeholder,
	/// Google's own, or anything else this gateway didn't write.
	Foreign,
}

pub(crate) fn read_signature(signature: &str) -> Signature {
	if PLACEHOLDERS.contains(&signature) {
		return Signature::Placeholder;
	}
	let standard: String = signature
		.chars()
		.map(|char| match char {
			'-' => '+',
			'_' => '/',
			other => other,
		})
		.collect();
	let Ok(bytes) = LENIENT.decode(standard) else {
		return Signature::Foreign;
	};
	let Ok(text) = String::from_utf8(bytes) else {
		return Signature::Foreign;
	};
	let carriers: Vec<String> = text.split(' ').map(str::to_string).collect();
	if carriers.iter().all(|carrier| carrier.starts_with(PREFIX)) {
		Signature::Carriers(carriers)
	} else {
		Signature::Foreign
	}
}

#[cfg(test)]
mod tests {
	#![allow(clippy::panic)]

	use base64::engine::general_purpose::URL_SAFE_NO_PAD;
	use rstest::rstest;

	use super::*;

	fn carriers(signature: &str) -> Vec<String> {
		match read_signature(signature) {
			Signature::Carriers(carriers) => carriers,
			Signature::Placeholder => panic!("a placeholder"),
			Signature::Foreign => panic!("foreign"),
		}
	}

	#[test]
	fn carriers_come_back_in_order_from_either_alphabet() {
		// Bytes chosen so the standard alphabet needs `+` and `/`.
		let sent = vec![format!("{PREFIX}a>?b"), format!("{PREFIX}c~~d")];
		let signature = write_signature(&sent);
		assert!(signature.contains('+') || signature.contains('/'));
		assert_eq!(carriers(&signature), sent);
		let url_safe = URL_SAFE_NO_PAD.encode(sent.join(" "));
		assert_eq!(carriers(&url_safe), sent);
	}

	#[rstest]
	#[case::googles(String::from("CjEBc2lnbmF0dXJlLWZvci10aGUtY2FsbA=="))]
	#[case::not_base64(String::from("not base64!"))]
	#[case::another_carrier_format(STANDARD.encode(format!("{PREFIX}a other")))]
	#[case::not_text(STANDARD.encode([0xff, 0xfe]))]
	fn a_signature_this_gateway_did_not_write_is_foreign(#[case] signature: String) {
		assert!(matches!(read_signature(&signature), Signature::Foreign));
	}

	#[test]
	fn a_placeholder_signature_is_recognized() {
		for signature in PLACEHOLDERS {
			assert!(
				matches!(read_signature(signature), Signature::Placeholder),
				"{signature}"
			);
		}
	}
}
