use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct JwtRecognizer {
	recognizer: PatternRecognizer,
}

impl JwtRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"JWT",
			vec![
				"jwt".to_string(),
				"token".to_string(),
				"bearer".to_string(),
				"authorization".to_string(),
			],
		);
		recognizer.add_pattern(
			"JWT (medium)",
			r"\beyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\b",
			0.8,
		);

		Self { recognizer }
	}
}

impl Recognizer for JwtRecognizer {
	fn recognize(&self, text: &str) -> Vec<super::recognizer_result::RecognizerResult> {
		self.recognizer.recognize(text)
	}
	fn name(&self) -> &str {
		self.recognizer.name()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_jwt_recognizer_detects_fake_jwt() {
		let recognizer = JwtRecognizer::new();
		let text = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.dQw4w9WgXcQ_fakefakefakefake";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
			assert_eq!(result.entity_type, "JWT");
		}
	}

	#[test]
	fn test_jwt_recognizer_detects_another_fake_jwt() {
		let recognizer = JwtRecognizer::new();
		let text =
			"token=eyJhbGciOiJIUzI1NiJ9.eyJ1c2VyIjoiYWxpY2UiLCJyb2xlIjoiYWRtaW4ifQ.abc123XYZ_-fakeSig";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		assert!(results[0].score > 0.0);
	}

	#[test]
	fn test_jwt_recognizer_no_match_on_plain_sentence() {
		let recognizer = JwtRecognizer::new();
		let text = "This is just a plain sentence with no tokens or secrets in it at all.";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}
}
