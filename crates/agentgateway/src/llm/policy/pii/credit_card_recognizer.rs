use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct CreditCardRecognizer {
	recognizer: PatternRecognizer,
}

impl CreditCardRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"CREDIT_CARD",
			vec![
				"credit".to_string(),
				"card".to_string(),
				"visa".to_string(),
				"mastercard".to_string(),
				"cc".to_string(),
				"amex".to_string(),
				"discover".to_string(),
				"jcb".to_string(),
				"diners".to_string(),
				"maestro".to_string(),
				"instapayment".to_string(),
			],
		);

		recognizer.add_pattern(
			"visa",
			r"\b4\d{3}[- ]?(\d{3,4})[- ]?(\d{3,4})[- ]?(\d{3,5})\b",
			0.3,
		);
		recognizer.add_pattern(
			"mastercard",
			r"\b5[0-5]\d{2}[- ]?(\d{3,4})[- ]?(\d{3,4})[- ]?(\d{3,5})\b",
			0.3,
		);
		recognizer.add_pattern(
			"discover",
			r"\b6\d{3}[- ]?(\d{3,4})[- ]?(\d{3,4})[- ]?(\d{3,5})\b",
			0.3,
		);
		recognizer.add_pattern(
			"amex",
			r"\b3\d{3}[- ]?(\d{3,4})[- ]?(\d{3,4})[- ]?(\d{3,5})\b",
			0.3,
		);
		// For Diners Club (1xxx), we need to be more specific to avoid 13-digit matches
		recognizer.add_pattern(
			"diners",
			r"\b1\d{3}[- ]?(\d{3,4})[- ]?(\d{3,4})[- ]?(\d{4,5})\b",
			0.3,
		);

		Self { recognizer }
	}
}

impl Recognizer for CreditCardRecognizer {
	fn recognize(&self, text: &str) -> Vec<super::recognizer_result::RecognizerResult> {
		self.recognizer.recognize(text)
	}
	fn name(&self) -> &str {
		self.recognizer.name()
	}
}
