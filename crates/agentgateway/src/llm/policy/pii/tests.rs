use crate::llm::policy::pii::email_recognizer::EmailRecognizer;
use crate::llm::policy::pii::phone_recognizer::PhoneRecognizer;
use crate::llm::policy::pii::recognizer::Recognizer;
use crate::llm::policy::pii::url_recognizer::UrlRecognizer;
use crate::llm::policy::pii::{recognizer_result, *};

#[test]
fn test_recognize() {
	let text = "Contact us at support@example.com, call (123) 456-7890, or visit https://example.com for more info. Or try info@domain.org, +1-800-555-1234, and http://another-site.org.";

	// Create recognizers
	let url_recognizer = UrlRecognizer::new();
	let email_recognizer = EmailRecognizer::new();
	let phone_recognizer = PhoneRecognizer::new();

	// Use trait objects for polymorphism
	let recognizers: Vec<&dyn Recognizer> =
		vec![&url_recognizer, &email_recognizer, &phone_recognizer];

	for recognizer in recognizers {
		let results = recognizer.recognize(text);
		println!("Results for {}:", recognizer.name());
		for result in results {
			println!(
				"  [{}-{}] {} (score: {})",
				result.start, result.end, result.matched, result.score
			);
		}
	}
}

#[test]
fn test_email_recognizer() {
	let recognizer = EmailRecognizer::new();

	// Test valid email addresses
	let text = "Contact us at test@example.com or support@domain.org";
	let results = recognizer.recognize(text);

	assert_eq!(results.len(), 2);
	assert_eq!(results[0].matched, "test@example.com");
	assert_eq!(results[1].matched, "support@domain.org");
	assert!(results[0].score > 0.0);
	assert!(results[1].score > 0.0);
}

#[test]
fn test_phone_recognizer() {
	let recognizer = PhoneRecognizer::new();

	// Test various phone number formats
	let text = "Call us at (123) 456-7890 or +1-800-555-1234 or 555.123.4567";
	let results = recognizer.recognize(text);

	assert!(results.len() >= 3);
	// Check that we found phone numbers
	let matched_numbers: Vec<&str> = results.iter().map(|r| r.matched.as_str()).collect();
	assert!(
		matched_numbers
			.iter()
			.any(|&s| s.contains("(123) 456-7890"))
	);
	assert!(
		matched_numbers
			.iter()
			.any(|&s| s.contains("+1-800-555-1234"))
	);
}

#[test]
fn test_url_recognizer() {
	let recognizer = UrlRecognizer::new();

	// Test various URL formats
	let text = "Visit https://example.com or http://another-site.org or www.test.com";
	let results = recognizer.recognize(text);

	assert!(results.len() >= 2);
	// Check that we found URLs
	let matched_urls: Vec<&str> = results.iter().map(|r| r.matched.as_str()).collect();
	assert!(
		matched_urls
			.iter()
			.any(|&s| s.contains("https://example.com"))
	);
	assert!(
		matched_urls
			.iter()
			.any(|&s| s.contains("http://another-site.org"))
	);
}

#[test]
fn test_credit_card_recognizer() {
	let recognizer = credit_card_recognizer::CreditCardRecognizer::new();

	// Test credit card numbers (using test numbers)
	let text = "Card number: 4111-1111-1111-1111 or 5555-5555-5555-4444";
	let results = recognizer.recognize(text);

	// Should find credit card patterns
	assert!(!results.is_empty());
	for result in results {
		assert!(result.score > 0.0);
		assert!(result.matched.contains("1111") || result.matched.contains("5555"));
	}
}

#[test]
fn test_ssn_recognizer() {
	let recognizer = us_ssn_recognizer::UsSsnRecognizer::new();

	// Test SSN patterns (using test numbers)
	let text = "SSN: 123-45-6789 or 987-65-4321";
	let results = recognizer.recognize(text);

	// Should find SSN patterns
	assert!(!results.is_empty());
	for result in results {
		assert!(result.score > 0.0);
		assert!(result.matched.contains("-"));
	}
}

#[test]
fn test_pattern_recognizer() {
	let mut recognizer = pattern_recognizer::PatternRecognizer::new("TEST", vec!["test".to_string()]);
	recognizer.add_pattern("test", r"\btest\b", 1.0);
	let results = recognizer.recognize("this is a test string");
	assert_eq!(
		results,
		vec![recognizer_result::RecognizerResult {
			entity_type: "TEST".to_string(),
			matched: "test".to_string(),
			start: 10,
			end: 14,
			score: 1.0,
		}]
	);
}

#[test]
fn test_multiple_recognizers() {
	let text = "User: john.doe@example.com, Phone: (555) 123-4567, Website: https://example.com, Card: 4111-1111-1111-1111, SSN: 123-45-6789";

	let email_recognizer = EmailRecognizer::new();
	let phone_recognizer = PhoneRecognizer::new();
	let url_recognizer = UrlRecognizer::new();
	let cc_recognizer = credit_card_recognizer::CreditCardRecognizer::new();
	let ssn_recognizer = us_ssn_recognizer::UsSsnRecognizer::new();

	let recognizers: Vec<&dyn Recognizer> = vec![
		&email_recognizer,
		&phone_recognizer,
		&url_recognizer,
		&cc_recognizer,
		&ssn_recognizer,
	];

	let mut total_results = 0;
	for recognizer in recognizers {
		let results = recognizer.recognize(text);
		total_results += results.len();

		// Each recognizer should find at least one match
		assert!(
			!results.is_empty(),
			"{} found no matches",
			recognizer.name()
		);

		for result in results {
			assert!(
				result.score > 0.0,
				"Score should be positive for {}",
				recognizer.name()
			);
			assert!(
				!result.matched.is_empty(),
				"Match should not be empty for {}",
				recognizer.name()
			);
		}
	}

	// Should find multiple types of PII
	assert!(
		total_results >= 5,
		"Expected at least 5 total matches, got {total_results}"
	);
}
