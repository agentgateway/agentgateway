use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct SlackTokenRecognizer {
	recognizer: PatternRecognizer,
}

impl SlackTokenRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"SLACK_TOKEN",
			vec![
				"slack".to_string(),
				"token".to_string(),
				"webhook".to_string(),
			],
		);
		recognizer.add_pattern(
			"SLACK_API_TOKEN",
			r"\bxox[baprs]-[0-9A-Za-z-]{10,72}\b",
			0.85,
		);
		recognizer.add_pattern(
			"SLACK_WEBHOOK_URL",
			r"\bhttps://hooks\.slack\.com/services/[A-Za-z0-9]+/[A-Za-z0-9]+/[A-Za-z0-9]+\b",
			0.9,
		);

		Self { recognizer }
	}
}

impl Recognizer for SlackTokenRecognizer {
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
	fn test_slack_bot_token_detected() {
		let recognizer = SlackTokenRecognizer::new();
		// Deliberately non-standard segment shapes (real Slack tokens use
		// digit-digit-alnum groups) so this doesn't trip secret-scanning push
		// protection while still exercising our (intentionally lenient) pattern.
		let text =
			"Our slack token is xoxb-notarealteam-notarealbotuser-notarealsecretvalue, keep it secret.";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
		assert!(
			results
				.iter()
				.any(|r| r.matched.starts_with("xoxb-notarealteam"))
		);
	}

	#[test]
	fn test_slack_webhook_url_detected() {
		let recognizer = SlackTokenRecognizer::new();
		let text = "Send messages via this webhook: https://hooks.slack.com/services/notarealteam/notarealbotuser/notarealsecretvalue";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
		assert!(
			results
				.iter()
				.any(|r| r.matched.contains("hooks.slack.com/services"))
		);
	}

	#[test]
	fn test_plain_sentence_no_matches() {
		let recognizer = SlackTokenRecognizer::new();
		let text = "This is just a plain sentence with no secrets or tokens in it.";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}
}
