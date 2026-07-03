use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct GcpApiKeyRecognizer {
	recognizer: PatternRecognizer,
}

impl GcpApiKeyRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"GCP_API_KEY",
			vec![
				"gcp".to_string(),
				"google".to_string(),
				"api key".to_string(),
				"aiza".to_string(),
			],
		);
		recognizer.add_pattern("GCP_API_KEY (high)", r"\bAIza[0-9A-Za-z_-]{35}\b", 0.85);

		Self { recognizer }
	}
}

impl Recognizer for GcpApiKeyRecognizer {
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
	fn test_gcp_api_key_recognizer_detects_key() {
		let recognizer = GcpApiKeyRecognizer::new();

		// Fake GCP API key: "AIza" + 35 chars = 39 chars total.
		let fake_key = "AIzaSyABCDEFGHIJKLMNOPQRSTUVWXYZ0123456";
		assert_eq!(fake_key.len(), 39);

		let text = format!("My GCP API key is {} - do not share it.", fake_key);
		let results = recognizer.recognize(&text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
			assert_eq!(result.matched, fake_key);
		}
	}

	#[test]
	fn test_gcp_api_key_recognizer_no_match_on_plain_text() {
		let recognizer = GcpApiKeyRecognizer::new();

		let text = "This is just a plain sentence with no secrets in it at all.";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}
}
