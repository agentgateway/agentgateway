use crate::llm::policy::pii::pattern_recognizer::PatternRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;

pub struct PrivateKeyRecognizer {
	recognizer: PatternRecognizer,
}

impl PrivateKeyRecognizer {
	pub fn new() -> Self {
		let mut recognizer = PatternRecognizer::new(
			"PRIVATE_KEY",
			vec![
				"private".to_string(),
				"key".to_string(),
				"rsa".to_string(),
				"openssh".to_string(),
				"pem".to_string(),
				"pgp".to_string(),
			],
		);

		// Generic PEM private key block, e.g. RSA, EC, DSA, OPENSSH, ENCRYPTED, or plain
		// "-----BEGIN PRIVATE KEY-----" ... "-----END PRIVATE KEY-----".
		recognizer.add_pattern(
			"PEM_PRIVATE_KEY_BLOCK",
			r"(?s)-----BEGIN (?:[A-Z0-9]+ )?PRIVATE KEY-----.*?-----END (?:[A-Z0-9]+ )?PRIVATE KEY-----",
			0.95,
		);

		// PGP private key block.
		recognizer.add_pattern(
			"PGP_PRIVATE_KEY_BLOCK",
			r"(?s)-----BEGIN PGP PRIVATE KEY BLOCK-----.*?-----END PGP PRIVATE KEY BLOCK-----",
			0.95,
		);

		Self { recognizer }
	}
}

impl Recognizer for PrivateKeyRecognizer {
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
	fn test_rsa_pem_private_key_block() {
		let recognizer = PrivateKeyRecognizer::new();

		let text = "Here is a key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIFakeBASE64garbageDATAxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\nAAAAfakefakefakefakefakefakefakefakefakefakefakefakefakefakefa\nkefakefakefakefakefakefakefakefakefakefakefakefakefakefake==\n-----END RSA PRIVATE KEY-----\nend of message";

		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
			assert!(result.matched.contains("BEGIN"));
			assert!(result.matched.contains("END"));
		}
	}

	#[test]
	fn test_openssh_private_key_block() {
		let recognizer = PrivateKeyRecognizer::new();

		let text = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1mYWtlZmFrZWZha2VmYWtlZmFrZWZha2VmYWtlZmFrZWZha2VmYWtl\nZmFrZWZha2VmYWtlZmFrZWZha2VmYWtlZmFrZWZha2VmYWtlZmFrZWZha2VmYWtl\n-----END OPENSSH PRIVATE KEY-----";

		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
			assert!(result.matched.contains("BEGIN"));
			assert!(result.matched.contains("END"));
		}
	}

	#[test]
	fn test_pgp_private_key_block() {
		let recognizer = PrivateKeyRecognizer::new();

		let text = "-----BEGIN PGP PRIVATE KEY BLOCK-----\nVersion: FakeGPG v1\n\nlQFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFake\nFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFakeFake==\n-----END PGP PRIVATE KEY BLOCK-----";

		let results = recognizer.recognize(text);

		assert!(!results.is_empty());
		for result in &results {
			assert!(result.score > 0.0);
			assert!(result.matched.contains("BEGIN"));
			assert!(result.matched.contains("END"));
		}
	}

	#[test]
	fn test_no_false_positive_on_plain_text() {
		let recognizer = PrivateKeyRecognizer::new();

		let text = "This is just a normal sentence about my private key management process.";
		let results = recognizer.recognize(text);

		assert!(results.is_empty());
	}
}
