use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct AwsAccessKeyRecognizer {
	recognizer: PatternRecognizer,
}

impl AwsAccessKeyRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"AWS_ACCESS_KEY",
			vec![
				"aws".to_string(),
				"amazon".to_string(),
				"access key".to_string(),
				"akid".to_string(),
				"access_key_id".to_string(),
			],
		);
		// Standard long-term access key ID (e.g. AKIA...)
		recognizer.add_pattern("AWS_ACCESS_KEY_STANDARD", r"\bAKIA[0-9A-Z]{16}\b", 0.85);
		// Temporary/STS access key ID (e.g. ASIA...)
		recognizer.add_pattern("AWS_ACCESS_KEY_TEMPORARY", r"\bASIA[0-9A-Z]{16}\b", 0.85);

		Self { recognizer }
	}
}

impl Recognizer for AwsAccessKeyRecognizer {
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
	fn test_aws_access_key_standard_detected() {
		let recognizer = AwsAccessKeyRecognizer::new();
		let text = "My aws access key is AKIA1234567890ABCDEF, keep it secret.";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
			assert!(result.matched.starts_with("AKIA"));
		}
		assert!(results.iter().any(|r| r.matched == "AKIA1234567890ABCDEF"));
	}

	#[test]
	fn test_aws_access_key_temporary_detected() {
		let recognizer = AwsAccessKeyRecognizer::new();
		let text = "Temporary access_key_id: ASIA1234567890ABCDEF for STS session.";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
			assert!(result.matched.starts_with("ASIA"));
		}
		assert!(results.iter().any(|r| r.matched == "ASIA1234567890ABCDEF"));
	}

	#[test]
	fn test_plain_sentence_no_matches() {
		let recognizer = AwsAccessKeyRecognizer::new();
		let text = "This is just a plain sentence with no secrets in it at all.";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}
}
