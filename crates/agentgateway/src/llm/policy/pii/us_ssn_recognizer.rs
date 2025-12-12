use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct UsSsnRecognizer {
	recognizer: PatternRecognizer,
}

impl UsSsnRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"SSN",
			vec![
				"social".to_string(),
				"security".to_string(),
				"ssn".to_string(),
				"ssns".to_string(),
				"ssid".to_string(),
			],
		);
		recognizer.add_pattern("SSN1 (very weak)", r"\b([0-9]{5})-([0-9]{4})\b", 0.05);
		recognizer.add_pattern("SSN2 (very weak)", r"\b([0-9]{3})-([0-9]{6})\b", 0.05);
		recognizer.add_pattern(
			"SSN3 (very weak)",
			r"\b(([0-9]{3})-([0-9]{2})-([0-9]{4}))\b",
			0.05,
		);
		recognizer.add_pattern("SSN4 (very weak)", r"\b[0-9]{9}\b", 0.05);
		recognizer.add_pattern(
			"SSN5 (medium)",
			r"\b([0-9]{3})[- .]([0-9]{2})[- .]([0-9]{4})\b",
			0.5,
		);

		Self { recognizer }
	}
}

impl Recognizer for UsSsnRecognizer {
	fn recognize(&self, text: &str) -> Vec<super::recognizer_result::RecognizerResult> {
		self.recognizer.recognize(text)
	}
	fn name(&self) -> &str {
		self.recognizer.name()
	}
}
