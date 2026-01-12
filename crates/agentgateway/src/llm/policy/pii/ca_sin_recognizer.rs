use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct CaSinRecognizer {
	recognizer: PatternRecognizer,
}

impl CaSinRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"CA_SIN",
			vec!["sin".to_string(), "social insurance number".to_string()],
		);
		// Match formatted SINs with hyphens or spaces (higher confidence)
		recognizer.add_pattern(
			"CA_SIN_FORMATTED",
			r"\b([0-9]{3})[- ]([0-9]{3})[- ]([0-9]{3})\b",
			0.7,
		);
		// Match unformatted SINs (lower confidence as it could be other 9-digit numbers)
		recognizer.add_pattern("CA_SIN_UNFORMATTED", r"\b[0-9]{9}\b", 0.3);

		Self { recognizer }
	}
}

impl Recognizer for CaSinRecognizer {
	fn recognize(&self, text: &str) -> Vec<super::recognizer_result::RecognizerResult> {
		self.recognizer.recognize(text)
	}
	fn name(&self) -> &str {
		self.recognizer.name()
	}
}
