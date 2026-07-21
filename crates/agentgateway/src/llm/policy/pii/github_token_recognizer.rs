use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct GithubTokenRecognizer {
	recognizer: PatternRecognizer,
}

impl GithubTokenRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"GITHUB_TOKEN",
			vec![
				"github".to_string(),
				"token".to_string(),
				"pat".to_string(),
				"gh".to_string(),
			],
		);
		// Personal access token (classic)
		recognizer.add_pattern("GITHUB_PAT_CLASSIC", r"\bghp_[A-Za-z0-9]{36}\b", 0.9);
		// OAuth token
		recognizer.add_pattern("GITHUB_OAUTH_TOKEN", r"\bgho_[A-Za-z0-9]{36}\b", 0.9);
		// User-to-server token
		recognizer.add_pattern(
			"GITHUB_USER_TO_SERVER_TOKEN",
			r"\bghu_[A-Za-z0-9]{36}\b",
			0.9,
		);
		// Server-to-server token
		recognizer.add_pattern(
			"GITHUB_SERVER_TO_SERVER_TOKEN",
			r"\bghs_[A-Za-z0-9]{36}\b",
			0.9,
		);
		// Refresh token
		recognizer.add_pattern("GITHUB_REFRESH_TOKEN", r"\bghr_[A-Za-z0-9]{36}\b", 0.9);
		// Fine-grained personal access token
		recognizer.add_pattern(
			"GITHUB_PAT_FINE_GRAINED",
			r"\bgithub_pat_[A-Za-z0-9_]{60,}\b",
			0.9,
		);

		Self { recognizer }
	}
}

impl Recognizer for GithubTokenRecognizer {
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
	fn test_github_pat_classic() {
		let recognizer = GithubTokenRecognizer::new();
		let text = "My token is ghp_abcdefghij1234567890ABCDEFGHIJ123456 please keep it secret";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
		assert!(
			results
				.iter()
				.any(|r| r.matched == "ghp_abcdefghij1234567890ABCDEFGHIJ123456")
		);
	}

	#[test]
	fn test_github_oauth_token() {
		let recognizer = GithubTokenRecognizer::new();
		let text = "gho_abcdefghij1234567890ABCDEFGHIJ123456 is an oauth token";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
	}

	#[test]
	fn test_github_fine_grained_pat() {
		let recognizer = GithubTokenRecognizer::new();
		let text = "github_pat_11ABCDEFGHIJKLMNOPQRST_abcdefghij1234567890ABCDEFGHIJ1234567890abcdefghijklmno is a fine-grained pat";
		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
		}
		assert!(results.iter().any(|r| r.matched.starts_with("github_pat_")));
	}

	#[test]
	fn test_no_false_positive_on_plain_text() {
		let recognizer = GithubTokenRecognizer::new();
		let text = "The quick brown fox jumps over the lazy dog near the github repository page.";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}

	#[test]
	fn test_no_match_on_short_token_like_string() {
		let recognizer = GithubTokenRecognizer::new();
		// Too short to be a valid token, should not match.
		let text = "ghp_short";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}
}
