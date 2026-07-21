use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct ApiKeyRecognizer {
	recognizer: PatternRecognizer,
}

impl ApiKeyRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"API_KEY",
			vec![
				"api".to_string(),
				"key".to_string(),
				"apikey".to_string(),
				"api_key".to_string(),
				"api-key".to_string(),
				"secret".to_string(),
				"token".to_string(),
			],
		);

		// OpenAI-style secret keys (sk- prefix followed by a long alphanumeric string).
		recognizer.add_pattern("OpenAI secret key", r"\bsk-[A-Za-z0-9]{20,}\b", 0.6);

		// Stripe live secret key (sk_live_ prefix).
		recognizer.add_pattern(
			"Stripe live secret key",
			r"\bsk_live_[A-Za-z0-9]{16,}\b",
			0.7,
		);

		// Stripe test secret key (sk_test_ prefix).
		recognizer.add_pattern(
			"Stripe test secret key",
			r"\bsk_test_[A-Za-z0-9]{16,}\b",
			0.6,
		);

		// Generic api_key/apikey/api-key assignment, e.g. api_key="abcdef0123456789..." or apikey: 'abc...'
		recognizer.add_pattern(
			"Generic API key assignment (weak)",
			r#"(?i)\bapi[_-]?key\b\s*[:=]\s*['"]?[A-Za-z0-9_\-]{16,}['"]?"#,
			0.4,
		);

		// Generic Bearer token in an Authorization header.
		recognizer.add_pattern(
			"Bearer token (weak)",
			r"(?i)\bBearer\s+[A-Za-z0-9\-_.=]{20,}\b",
			0.4,
		);

		Self { recognizer }
	}
}

impl Recognizer for ApiKeyRecognizer {
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
	fn test_openai_style_key_detected() {
		let recognizer = ApiKeyRecognizer::new();

		let text = "My OpenAI key is sk-abcdefghijklmnopqrstuvwxyz123456 please don't share it.";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
		assert!(
			results
				.iter()
				.any(|r| r.matched.contains("sk-abcdefghijklmnopqrstuvwxyz123456"))
		);
	}

	#[test]
	fn test_stripe_style_key_detected() {
		let recognizer = ApiKeyRecognizer::new();

		// Deliberately low-entropy/repetitive placeholder (not a real Stripe key
		// shape) so it doesn't trip GitHub's secret-scanning push protection.
		let text = "Stripe secret: sk_test_AAAABBBBCCCCDDDD use in tests only.";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
		assert!(
			results
				.iter()
				.any(|r| r.matched.contains("sk_test_AAAABBBBCCCCDDDD"))
		);
	}

	#[test]
	fn test_generic_api_key_assignment_detected() {
		let recognizer = ApiKeyRecognizer::new();

		let text = r#"config.api_key = "FAKE1234567890ABCDEFGHIJ""#;
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
	}

	#[test]
	fn test_bearer_token_detected() {
		let recognizer = ApiKeyRecognizer::new();

		let text = "Authorization: Bearer FAKEabcdefghijklmnopqrstuvwxyz0123456789";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
	}

	#[test]
	fn test_plain_text_no_false_positive() {
		let recognizer = ApiKeyRecognizer::new();

		let text = "This is just a normal sentence with no secrets in it.";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}
}
