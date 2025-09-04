use http::Uri;

use super::*;

#[test]
fn test_build_agent_path() {
	let test_cases = vec![
		// Test stripping /.well-known/agent.json
		(
			"https://example.com/.well-known/agent.json",
			"https://example.com",
		),
		(
			"https://example.com/api/.well-known/agent.json",
			"https://example.com/api",
		),
		(
			"http://localhost:8080/service/.well-known/agent.json",
			"http://localhost:8080/service",
		),
		// Test stripping /.well-known/agent-card.json
		(
			"https://example.com/.well-known/agent-card.json",
			"https://example.com",
		),
		(
			"https://example.com/api/.well-known/agent-card.json",
			"https://example.com/api",
		),
		(
			"http://localhost:8080/service/.well-known/agent-card.json",
			"http://localhost:8080/service",
		),
		(
			"https://example.com:443/.well-known/agent.json",
			"https://example.com:443",
		),
		(
			"http://example.com:80/.well-known/agent-card.json",
			"http://example.com:80",
		),
	];

	for (input_url, expected_output) in test_cases {
		let uri: Uri = input_url.parse().expect("Failed to parse URI");
		let result = build_agent_path(uri);
		assert_eq!(result, expected_output, "Failed for input: {}", input_url);
	}
}
