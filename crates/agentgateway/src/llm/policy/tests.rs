use ::http::{HeaderName, HeaderValue};

use super::*;
use crate::types::agent::HeaderValueMatch;

/// When a webhook guard fails open, exactly one metric must be emitted (`FailOpen`); the caller
/// must not additionally record `Allow`.
#[tokio::test]
async fn webhook_fail_open_emits_single_metric() {
	use crate::telemetry::metrics::{GuardrailAction, GuardrailLabels, GuardrailPhase};
	use crate::types::agent::SimpleBackendReference;

	let guard = PromptGuard {
		streaming: Default::default(),
		request: vec![RequestGuard {
			rejection: Default::default(),
			kind: RequestGuardKind::Webhook(Webhook {
				target: SimpleBackendReference::Invalid,
				headers: Default::default(),
				forward_header_matches: vec![],
				failure_mode: FailureMode::FailOpen,
			}),
		}],
		response: vec![],
	};

	let client = crate::test_helpers::policy_client();
	let blocked = guard
		.apply_realtime_request_guards("hello world", &client, None)
		.await;
	assert!(blocked.is_none(), "FailOpen must not block the request");

	let fail_open = client
		.inputs
		.metrics
		.guardrail_checks
		.get_or_create(&GuardrailLabels {
			phase: GuardrailPhase::Request,
			action: GuardrailAction::FailOpen,
		})
		.get();
	let allow = client
		.inputs
		.metrics
		.guardrail_checks
		.get_or_create(&GuardrailLabels {
			phase: GuardrailPhase::Request,
			action: GuardrailAction::Allow,
		})
		.get();

	assert_eq!(fail_open, 1, "FailOpen should be recorded exactly once");
	assert_eq!(
		allow, 0,
		"Allow must not be recorded for a FailOpen outcome"
	);
}

#[test]
fn test_get_webhook_forward_headers() {
	let mut headers = HeaderMap::new();
	headers.append("x-test-header", HeaderValue::from_static("first-value"));
	headers.append("x-test-header", HeaderValue::from_static("test-value"));
	headers.insert(
		"x-another-header",
		HeaderValue::from_static("another-value"),
	);
	headers.insert(
		"x-regex-header",
		HeaderValue::from_static("regex-match-123"),
	);
	headers.insert(
		"x-comma-header",
		HeaderValue::from_static("first-value,test-value"),
	);

	let header_matches = vec![
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-test-header")),
			value: HeaderValueMatch::Exact(HeaderValue::from_static("test-value")),
		},
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-another-header")),
			value: HeaderValueMatch::Exact(HeaderValue::from_static("wrong-value")),
		},
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-regex-header")),
			value: HeaderValueMatch::Regex(regex::Regex::new(r"regex-match-\d+").unwrap()),
		},
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-missing-header")),
			value: HeaderValueMatch::Exact(HeaderValue::from_static("some-value")),
		},
		HeaderMatch {
			name: crate::http::HeaderOrPseudo::Header(HeaderName::from_static("x-comma-header")),
			value: HeaderValueMatch::Exact(HeaderValue::from_static("test-value")),
		},
	];

	let result = Policy::get_webhook_forward_headers(&headers, &header_matches);

	assert_eq!(result.len(), 3);
	assert_eq!(
		result
			.get_all("x-test-header")
			.iter()
			.cloned()
			.collect::<Vec<_>>(),
		vec![
			HeaderValue::from_static("first-value"),
			HeaderValue::from_static("test-value"),
		]
	);
	assert_eq!(
		result.get("x-regex-header").unwrap(),
		&HeaderValue::from_static("regex-match-123")
	);
	assert!(!result.contains_key("x-comma-header"));
}

#[test]
fn test_rejection_with_json_headers() {
	let rejection = RequestRejection {
		body: Bytes::from(r#"{"error": {"message": "test", "type": "invalid_request_error"}}"#),
		status: StatusCode::BAD_REQUEST,
		headers: Some(HeaderModifier {
			set: vec![
				(strng::new("content-type"), strng::new("application/json")),
				(strng::new("x-custom-header"), strng::new("custom-value")),
			],
			add: vec![],
			remove: vec![],
		}),
	};

	let response = rejection.as_response();
	assert_eq!(response.status(), StatusCode::BAD_REQUEST);
	assert_eq!(
		response.headers().get("content-type").unwrap(),
		"application/json"
	);
	assert_eq!(
		response.headers().get("x-custom-header").unwrap(),
		"custom-value"
	);
}

#[test]
fn test_rejection_add_multiple_header_values() {
	let rejection = RequestRejection {
		body: Bytes::from("blocked"),
		status: StatusCode::FORBIDDEN,
		headers: Some(HeaderModifier {
			set: vec![],
			add: vec![
				(strng::new("x-blocked-category"), strng::new("violence")),
				(strng::new("x-blocked-category"), strng::new("hate")),
			],
			remove: vec![],
		}),
	};

	let response = rejection.as_response();
	let values: Vec<_> = response
		.headers()
		.get_all("x-blocked-category")
		.iter()
		.map(|v| v.to_str().unwrap())
		.collect();
	assert_eq!(values, vec!["violence", "hate"]);
}

#[test]
fn test_rejection_backwards_compatibility() {
	// Simulate old config without headers field
	let rejection = RequestRejection {
		body: Bytes::from("error message"),
		status: StatusCode::FORBIDDEN,
		headers: None,
	};

	let response = rejection.as_response();
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	// Should have no extra headers
	assert!(response.headers().is_empty());
}

#[test]
fn test_rejection_default() {
	let rejection = RequestRejection::default();
	let response = rejection.as_response();
	assert_eq!(response.status(), StatusCode::FORBIDDEN);
	assert!(response.headers().is_empty());
}

#[test]
fn test_rejection_set_and_remove_headers() {
	let rejection = RequestRejection {
		body: Bytes::from("test"),
		status: StatusCode::BAD_REQUEST,
		headers: Some(HeaderModifier {
			set: vec![(strng::new("content-type"), strng::new("application/json"))],
			add: vec![],
			remove: vec![strng::new("server")],
		}),
	};

	let response = rejection.as_response();
	assert_eq!(
		response.headers().get("content-type").unwrap(),
		"application/json"
	);
	assert!(response.headers().get("server").is_none());
}

#[test]
fn test_prompt_caching_policy_deserialization() {
	use serde_json::json;

	let json = json!({
		"promptCaching": {
			"cacheSystem": true,
			"cacheMessages": true,
			"cacheTools": false,
			"minTokens": 1024
		}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();
	let caching = policy.prompt_caching.unwrap();

	assert!(caching.cache_system);
	assert!(caching.cache_messages);
	assert!(!caching.cache_tools);
	assert_eq!(caching.min_tokens, Some(1024));
	assert_eq!(caching.cache_message_offset, 0);
}

#[test]
fn test_prompt_caching_policy_defaults() {
	use serde_json::json;

	// Empty config should have system and messages enabled by default
	let json = json!({
		"promptCaching": {}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();
	let caching = policy.prompt_caching.unwrap();

	assert!(caching.cache_system); // Default: true
	assert!(caching.cache_messages); // Default: true
	assert!(!caching.cache_tools); // Default: false
	assert_eq!(caching.min_tokens, Some(1024)); // Default: 1024
	assert_eq!(caching.cache_message_offset, 0); // Default: 0
}

#[test]
fn test_policy_without_prompt_caching_field() {
	use serde_json::json;

	let json = json!({
		"modelAliases": {
			"gpt-4": "anthropic.claude-3-sonnet-20240229-v1:0"
		}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();

	// prompt_caching should be None when not specified
	assert!(policy.prompt_caching.is_none());
}

#[test]
fn test_prompt_caching_explicit_disable() {
	use serde_json::json;

	// Explicitly disable caching
	let json = json!({
		"promptCaching": null
	});

	let policy: Policy = serde_json::from_value(json).unwrap();

	// Should be None when explicitly set to null
	assert!(policy.prompt_caching.is_none());
}

#[test]
fn test_prompt_caching_with_offset() {
	use serde_json::json;

	let json = json!({
		"promptCaching": {
			"cacheMessages": true,
			"cacheMessageOffset": 4
		}
	});

	let policy: Policy = serde_json::from_value(json).unwrap();
	let caching = policy.prompt_caching.unwrap();

	assert!(caching.cache_messages);
	assert_eq!(caching.cache_message_offset, 4);
}

#[test]
fn test_resolve_route() {
	let mut routes = IndexMap::new();
	routes.insert(
		strng::literal!("/completions"),
		crate::llm::RouteType::Completions,
	);
	routes.insert(
		strng::literal!("/v1/messages"),
		crate::llm::RouteType::Messages,
	);
	routes.insert(
		strng::literal!("/v1/embeddings"),
		crate::llm::RouteType::Embeddings,
	);
	routes.insert(strng::literal!("*"), crate::llm::RouteType::Passthrough);

	let policy = Policy {
		routes: SortedRoutes::from_iter(routes.into_iter().map(|(k, v)| (strng::new(k), v))),
		..Default::default()
	};

	// Suffix matching
	assert_eq!(
		policy.resolve_route("/v1/chat/completions"),
		crate::llm::RouteType::Completions
	);
	assert_eq!(
		policy.resolve_route("/api/completions"),
		crate::llm::RouteType::Completions
	);
	// Exact suffix match
	assert_eq!(
		policy.resolve_route("/v1/messages"),
		crate::llm::RouteType::Messages
	);
	// Embeddings route
	assert_eq!(
		policy.resolve_route("/v1/embeddings"),
		crate::llm::RouteType::Embeddings
	);
	// Wildcard fallback
	assert_eq!(
		policy.resolve_route("/v1/models"),
		crate::llm::RouteType::Passthrough
	);
	// Empty routes defaults to Completions
	assert_eq!(
		Policy::default().resolve_route("/any/path"),
		crate::llm::RouteType::Completions
	);
}

#[test]
fn test_model_alias_wildcard_resolution() {
	let mut policy = Policy {
		model_aliases: HashMap::from([
			(strng::new("gpt-4"), strng::new("exact-target")),
			(
				strng::new("claude-haiku-3.5-*"),
				strng::new("haiku-3.5-target"),
			),
			(strng::new("claude-haiku-*"), strng::new("haiku-target")),
			(strng::new("*-sonnet-*"), strng::new("sonnet-target")),
		]),
		..Default::default()
	};

	policy.compile_model_alias_patterns();

	// Exact match takes precedence over wildcards
	assert_eq!(
		policy.resolve_model_alias("gpt-4"),
		Some(&strng::new("exact-target"))
	);

	// Longer patterns are more specific (checked first)
	assert_eq!(
		policy.resolve_model_alias("claude-haiku-3.5-v1"),
		Some(&strng::new("haiku-3.5-target")) // Matches "claude-haiku-3.5-*" not "claude-haiku-*"
	);
	assert_eq!(
		policy.resolve_model_alias("claude-haiku-v1"),
		Some(&strng::new("haiku-target")) // Only matches "claude-haiku-*"
	);
	assert_eq!(
		policy.resolve_model_alias("other-sonnet-model"),
		Some(&strng::new("sonnet-target")) // Matches "*-sonnet-*"
	);

	// No match returns None
	assert_eq!(policy.resolve_model_alias("unmatched-model"), None);
}

#[test]
fn test_model_alias_pattern_validation() {
	// Pattern must contain wildcard
	assert!(ModelAliasPattern::from_wildcard("no-wildcards").is_err());

	// Special characters are escaped (dot is literal, not regex wildcard)
	let pattern = ModelAliasPattern::from_wildcard("test.*").unwrap();
	assert!(pattern.matches("test.v1"));
	assert!(!pattern.matches("testXv1")); // X doesn't match literal dot
}

// ============================================================================
// Bedrock Guardrails Tests
// ============================================================================

mod bedrock_guardrails_tests {
	use serde_json::json;

	use super::super::bedrock_guardrails::*;

	#[test]
	fn test_apply_guardrail_response_is_blocked_true() {
		let json = json!({
			"action": "GUARDRAIL_INTERVENED",
			"outputs": [{"text": "Sorry, I can't help with that."}],
			"assessments": [{
				"contentPolicy": {
					"filters": [{
						"action": "BLOCKED",
						"type": "HATE",
						"confidence": "HIGH"
					}]
				}
			}]
		});
		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
		assert!(!response.is_anonymized());
	}

	#[test]
	fn test_apply_guardrail_response_is_blocked_false() {
		let json = json!({
			"action": "NONE",
			"outputs": [{"text": "Hello, world!"}],
			"assessments": [{}]
		});
		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
		assert!(!response.is_anonymized());
	}

	#[test]
	fn test_apply_guardrail_request_serialization() {
		let request = ApplyGuardrailRequest {
			source: GuardrailSource::Input,
			content: vec![GuardrailContentBlock {
				text: GuardrailTextBlock {
					text: "Hello, world!".to_string(),
				},
			}],
		};

		let serialized = serde_json::to_value(&request).unwrap();
		assert_eq!(serialized["source"], "INPUT");
		assert_eq!(serialized["content"][0]["text"]["text"], "Hello, world!");
	}

	#[test]
	fn test_apply_guardrail_request_multiple_content_blocks() {
		let request = ApplyGuardrailRequest {
			source: GuardrailSource::Output,
			content: vec![
				GuardrailContentBlock {
					text: GuardrailTextBlock {
						text: "First message".to_string(),
					},
				},
				GuardrailContentBlock {
					text: GuardrailTextBlock {
						text: "Second message".to_string(),
					},
				},
			],
		};

		let serialized = serde_json::to_value(&request).unwrap();
		assert_eq!(serialized["source"], "OUTPUT");
		assert_eq!(serialized["content"].as_array().unwrap().len(), 2);
		assert_eq!(serialized["content"][0]["text"]["text"], "First message");
		assert_eq!(serialized["content"][1]["text"]["text"], "Second message");
	}

	#[test]
	fn test_apply_guardrail_response_roundtrip() {
		// Simulate a realistic AWS Bedrock Guardrails API response
		let json = json!({
			"action": "GUARDRAIL_INTERVENED",
			"outputs": [{"text": "I can't help with that request."}],
			"assessments": [{
				"topicPolicy": {
					"topics": [{
						"action": "BLOCKED",
						"name": "Finance",
						"type": "DENY"
					}]
				}
			}],
			"usage": {
				"topicPolicyUnits": 1,
				"contentPolicyUnits": 0,
				"wordPolicyUnits": 0
			}
		});

		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
		assert!(!response.is_anonymized());
		assert_eq!(response.action, GuardrailAction::GuardrailIntervened);
	}

	#[test]
	fn test_apply_guardrail_response_anonymized() {
		let json = json!({
			"action": "GUARDRAIL_INTERVENED",
			"outputs": [{"text": "My name is {NAME} and my email is {EMAIL}"}],
			"assessments": [{
				"sensitiveInformationPolicy": {
					"piiEntities": [
						{
							"action": "ANONYMIZED",
							"match": "John Doe",
							"type": "NAME"
						},
						{
							"action": "ANONYMIZED",
							"match": "john@example.com",
							"type": "EMAIL"
						}
					]
				}
			}]
		});
		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
		assert!(response.is_anonymized());
		assert_eq!(
			response.output_texts(),
			vec!["My name is {NAME} and my email is {EMAIL}"]
		);
	}

	#[test]
	fn test_apply_guardrail_response_mixed_block_and_anonymize() {
		let json = json!({
			"action": "GUARDRAIL_INTERVENED",
			"outputs": [{"text": "blocked"}],
			"assessments": [{
				"sensitiveInformationPolicy": {
					"piiEntities": [{
						"action": "ANONYMIZED",
						"match": "John Doe",
						"type": "NAME"
					}]
				},
				"contentPolicy": {
					"filters": [{
						"action": "BLOCKED",
						"type": "HATE",
						"confidence": "HIGH"
					}]
				}
			}]
		});
		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
		assert!(!response.is_anonymized());
	}

	#[test]
	fn test_apply_guardrail_response_output_texts() {
		let json = json!({
			"action": "GUARDRAIL_INTERVENED",
			"outputs": [
				{"text": "First message with {NAME}"},
				{"text": "Second message with {EMAIL}"}
			],
			"assessments": [{
				"sensitiveInformationPolicy": {
					"piiEntities": [{
						"action": "ANONYMIZED",
						"match": "test",
						"type": "NAME"
					}]
				}
			}]
		});
		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_anonymized());
		assert_eq!(
			response.output_texts(),
			vec!["First message with {NAME}", "Second message with {EMAIL}"]
		);
	}

	#[test]
	fn test_apply_guardrail_response_intervened_no_assessments() {
		let json = json!({
			"action": "GUARDRAIL_INTERVENED",
			"outputs": [{"text": "modified content"}],
			"assessments": []
		});
		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
		assert!(response.is_anonymized());
	}
}

// ============================================================================
// Google Model Armor Tests
// ============================================================================

mod google_model_armor_tests {
	use serde_json::json;

	use super::super::google_model_armor::*;

	#[test]
	fn test_match_state_deserialization() {
		let json = json!("MATCH_FOUND");
		let state: MatchState = serde_json::from_value(json).unwrap();
		assert_eq!(state, MatchState::MatchFound);

		let json = json!("NO_MATCH_FOUND");
		let state: MatchState = serde_json::from_value(json).unwrap();
		assert_eq!(state, MatchState::NoMatchFound);

		// Unknown values should deserialize to Unknown
		let json = json!("SOME_NEW_STATE");
		let state: MatchState = serde_json::from_value(json).unwrap();
		assert_eq!(state, MatchState::Unknown);
	}

	#[test]
	fn test_sanitize_response_empty_is_not_blocked() {
		let response = SanitizeResponse::default();
		assert!(!response.is_blocked());

		let json = json!({});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_no_matches_is_not_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": []
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_rai_filter_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"raiFilterResult": {
						"matchState": "MATCH_FOUND"
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_pi_jailbreak_filter_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"piAndJailbreakFilterResult": {
						"matchState": "MATCH_FOUND"
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_malicious_uri_filter_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"maliciousUriFilterResult": {
						"matchState": "MATCH_FOUND"
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_csam_filter_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"csamFilterResult": {
						"matchState": "MATCH_FOUND"
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_virus_scan_filter_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"virusScanFilterResult": {
						"matchState": "MATCH_FOUND"
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_sdp_inspect_filter_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"sdpFilterResult": {
						"inspectResult": {
							"matchState": "MATCH_FOUND"
						}
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_sdp_deidentify_filter_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"sdpFilterResult": {
						"deidentifyResult": {
							"matchState": "MATCH_FOUND"
						}
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_no_match_found_is_not_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": [{
					"raiFilterResult": {
						"matchState": "NO_MATCH_FOUND"
					},
					"piAndJailbreakFilterResult": {
						"matchState": "NO_MATCH_FOUND"
					}
				}]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_filter_results_as_map() {
		// Test that FilterResults can be deserialized as a map (some API versions use this)
		let json = json!({
			"sanitizationResult": {
				"filterResults": {
					"filter1": {
						"raiFilterResult": {
							"matchState": "MATCH_FOUND"
						}
					}
				}
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_filter_results_map_not_blocked() {
		let json = json!({
			"sanitizationResult": {
				"filterResults": {
					"filter1": {
						"raiFilterResult": {
							"matchState": "NO_MATCH_FOUND"
						}
					},
					"filter2": {
						"piAndJailbreakFilterResult": {
							"matchState": "NO_MATCH_FOUND"
						}
					}
				}
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
	}

	#[test]
	fn test_sanitize_response_multiple_filters_one_blocked() {
		// Even if most filters pass, one MATCH_FOUND should block
		let json = json!({
			"sanitizationResult": {
				"filterResults": [
					{
						"raiFilterResult": {
							"matchState": "NO_MATCH_FOUND"
						}
					},
					{
						"piAndJailbreakFilterResult": {
							"matchState": "MATCH_FOUND"
						}
					},
					{
						"maliciousUriFilterResult": {
							"matchState": "NO_MATCH_FOUND"
						}
					}
				]
			}
		});
		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}

	#[test]
	fn test_sanitize_user_prompt_request_serialization() {
		let request = SanitizeUserPromptRequest {
			user_prompt_data: UserPromptData {
				text: "Hello, how are you?".to_string(),
			},
		};

		let serialized = serde_json::to_value(&request).unwrap();
		assert_eq!(
			serialized["user_prompt_data"]["text"],
			"Hello, how are you?"
		);
	}

	#[test]
	fn test_sanitize_model_response_request_serialization() {
		let request = SanitizeModelResponseRequest {
			model_response_data: ModelResponseData {
				text: "I'm doing well, thank you!".to_string(),
			},
		};

		let serialized = serde_json::to_value(&request).unwrap();
		assert_eq!(
			serialized["model_response_data"]["text"],
			"I'm doing well, thank you!"
		);
	}

	#[test]
	fn test_filter_results_entries_list() {
		let results = FilterResults::List(vec![
			FilterResultEntry::default(),
			FilterResultEntry::default(),
		]);
		assert_eq!(results.entries().len(), 2);
	}

	#[test]
	fn test_filter_results_entries_map() {
		let mut map = std::collections::HashMap::new();
		map.insert("filter1".to_string(), FilterResultEntry::default());
		map.insert("filter2".to_string(), FilterResultEntry::default());
		let results = FilterResults::Map(map);
		assert_eq!(results.entries().len(), 2);
	}

	#[test]
	fn test_realistic_model_armor_response() {
		// Simulate a realistic Google Model Armor API response
		let json = json!({
			"sanitizationResult": {
				"filterResults": [
					{
						"raiFilterResult": {
							"matchState": "NO_MATCH_FOUND",
							"raiFilterTypeResults": {}
						},
						"sdpFilterResult": {
							"inspectResult": {
								"matchState": "NO_MATCH_FOUND"
							}
						}
					}
				],
				"filterMatchState": "NO_MATCH_FOUND",
				"invocationResult": "SUCCESS"
			}
		});

		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
	}

	#[test]
	fn test_realistic_model_armor_blocked_response() {
		// Simulate a realistic Google Model Armor API response with content blocked
		let json = json!({
			"sanitizationResult": {
				"filterResults": [
					{
						"raiFilterResult": {
							"matchState": "MATCH_FOUND",
							"raiFilterTypeResults": {
								"dangerous": {
									"matchState": "MATCH_FOUND",
									"confidenceLevel": "HIGH"
								}
							}
						}
					}
				],
				"filterMatchState": "MATCH_FOUND",
				"invocationResult": "SUCCESS"
			}
		});

		let response: SanitizeResponse = serde_json::from_value(json).unwrap();
		assert!(response.is_blocked());
	}
}

// ============================================================================
// Prompt Guard Configuration Tests
// ============================================================================

mod prompt_guard_config_tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn test_bedrock_guardrails_config_deserialization() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"bedrockGuardrails": {
						"guardrailIdentifier": "my-guardrail-id",
						"guardrailVersion": "1",
						"region": "us-east-1",
						"policies": {
							"backendAuth": {
								"aws": {
									"accessKeyId": "AKIAIOSFODNN7EXAMPLE",
									"secretAccessKey": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
								}
							}
						}
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();
		assert_eq!(prompt_guard.request.len(), 1);

		match &prompt_guard.request[0].kind {
			RequestGuardKind::BedrockGuardrails(bg) => {
				assert_eq!(bg.guardrail_identifier.as_str(), "my-guardrail-id");
				assert_eq!(bg.guardrail_version.as_str(), "1");
				assert_eq!(bg.region.as_str(), "us-east-1");
				assert!(!bg.policies.is_empty());
			},
			_ => panic!("Expected BedrockGuardrails guard kind"),
		}
	}

	#[test]
	fn test_google_model_armor_config_deserialization() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"googleModelArmor": {
						"templateId": "my-template",
						"projectId": "my-project",
						"location": "us-central1",
						"policies": {
							"backendAuth": {
								"gcp": {}
							}
						}
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();
		assert_eq!(prompt_guard.request.len(), 1);

		match &prompt_guard.request[0].kind {
			RequestGuardKind::GoogleModelArmor(gma) => {
				assert_eq!(gma.template_id.as_str(), "my-template");
				assert_eq!(gma.project_id.as_str(), "my-project");
				assert_eq!(gma.location.as_ref().unwrap().as_str(), "us-central1");
				assert!(!gma.policies.is_empty());
			},
			_ => panic!("Expected GoogleModelArmor guard kind"),
		}
	}

	#[test]
	fn test_google_model_armor_config_default_location() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"googleModelArmor": {
						"templateId": "my-template",
						"projectId": "my-project",
						"policies": {
							"backendAuth": {
								"gcp": {}
							}
						}
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();

		match &prompt_guard.request[0].kind {
			RequestGuardKind::GoogleModelArmor(gma) => {
				// Location should be None when not specified (default applied at runtime)
				assert!(gma.location.is_none());
			},
			_ => panic!("Expected GoogleModelArmor guard kind"),
		}
	}

	#[test]
	fn test_response_guard_bedrock_guardrails() {
		let json = json!({
			"promptGuard": {
				"response": [{
					"bedrockGuardrails": {
						"guardrailIdentifier": "response-guardrail",
						"guardrailVersion": "2",
						"region": "eu-west-1",
						"policies": {
							"backendAuth": {
								"aws": {
									"accessKeyId": "AKIAIOSFODNN7EXAMPLE",
									"secretAccessKey": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
								}
							}
						}
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();
		assert_eq!(prompt_guard.response.len(), 1);
		assert!(prompt_guard.streaming.is_disabled());

		match &prompt_guard.response[0].kind {
			ResponseGuardKind::BedrockGuardrails(bg) => {
				assert_eq!(bg.guardrail_identifier.as_str(), "response-guardrail");
				assert_eq!(bg.guardrail_version.as_str(), "2");
				assert_eq!(bg.region.as_str(), "eu-west-1");
			},
			_ => panic!("Expected BedrockGuardrails response guard kind"),
		}
	}

	#[test]
	fn test_streaming_response_guards_are_opt_in() {
		let json = json!({
			"promptGuard": {
				"response": [{
					"regex": {
						"action": "reject",
						"rules": [{
							"pattern": "secret"
						}]
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();

		assert!(
			policy
				.prompt_guard
				.as_ref()
				.unwrap()
				.streaming
				.is_disabled()
		);
		assert!(!policy.has_streaming_response_guards());
	}

	#[test]
	fn test_streaming_response_guards_enable_with_streaming_true() {
		let json = json!({
			"promptGuard": {
				"streaming": "Enabled",
				"response": [{
					"regex": {
						"action": "reject",
						"rules": [{
							"pattern": "secret"
						}]
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();

		assert!(policy.prompt_guard.as_ref().unwrap().streaming.is_enabled());
		assert!(policy.has_streaming_response_guards());
	}

	#[test]
	fn test_response_guard_google_model_armor() {
		let json = json!({
			"promptGuard": {
				"response": [{
					"googleModelArmor": {
						"templateId": "response-template",
						"projectId": "my-project",
						"policies": {
							"backendAuth": {
								"gcp": {}
							}
						}
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();
		assert_eq!(prompt_guard.response.len(), 1);

		match &prompt_guard.response[0].kind {
			ResponseGuardKind::GoogleModelArmor(gma) => {
				assert_eq!(gma.template_id.as_str(), "response-template");
				assert_eq!(gma.project_id.as_str(), "my-project");
			},
			_ => panic!("Expected GoogleModelArmor response guard kind"),
		}
	}

	#[test]
	fn test_bedrock_guardrails_without_policies() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"bedrockGuardrails": {
						"guardrailIdentifier": "my-guardrail-id",
						"guardrailVersion": "DRAFT",
						"region": "us-west-2"
					}
				}],
				"response": [{
					"bedrockGuardrails": {
						"guardrailIdentifier": "my-guardrail-id",
						"guardrailVersion": "DRAFT",
						"region": "us-west-2"
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();
		assert_eq!(prompt_guard.request.len(), 1);
		assert_eq!(prompt_guard.response.len(), 1);

		match &prompt_guard.request[0].kind {
			RequestGuardKind::BedrockGuardrails(bg) => {
				assert!(bg.policies.is_empty());
			},
			_ => panic!("Expected BedrockGuardrails guard kind"),
		}
	}

	#[test]
	fn test_google_model_armor_without_policies() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"googleModelArmor": {
						"templateId": "my-template",
						"projectId": "my-project"
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();

		match &prompt_guard.request[0].kind {
			RequestGuardKind::GoogleModelArmor(gma) => {
				assert!(gma.policies.is_empty());
			},
			_ => panic!("Expected GoogleModelArmor guard kind"),
		}
	}

	#[test]
	fn test_mixed_guardrails_request_and_response() {
		let json = json!({
			"promptGuard": {
				"request": [
					{
						"googleModelArmor": {
							"templateId": "request-template",
							"projectId": "my-project",
							"policies": {
								"backendAuth": {
									"gcp": {}
								}
							}
						}
					}
				],
				"response": [
					{
						"bedrockGuardrails": {
							"guardrailIdentifier": "response-guardrail",
							"guardrailVersion": "1",
							"region": "us-west-2",
							"policies": {
								"backendAuth": {
									"aws": {
										"accessKeyId": "AKIAIOSFODNN7EXAMPLE",
										"secretAccessKey": "secret"
									}
								}
							}
						}
					}
				]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();

		assert_eq!(prompt_guard.request.len(), 1);
		assert_eq!(prompt_guard.response.len(), 1);

		assert!(matches!(
			&prompt_guard.request[0].kind,
			RequestGuardKind::GoogleModelArmor(_)
		));
		assert!(matches!(
			&prompt_guard.response[0].kind,
			ResponseGuardKind::BedrockGuardrails(_)
		));
	}

	#[test]
	fn test_guardrail_with_custom_rejection() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"rejection": {
						"body": "Content blocked by security policy",
						"status": 451
					},
					"bedrockGuardrails": {
						"guardrailIdentifier": "strict-guardrail",
						"guardrailVersion": "1",
						"region": "us-east-1",
						"policies": {
							"backendAuth": {
								"aws": {
									"accessKeyId": "AKIAIOSFODNN7EXAMPLE",
									"secretAccessKey": "secret"
								}
							}
						}
					}
				}]
			}
		});

		let policy: Policy = serde_json::from_value(json).unwrap();
		let prompt_guard = policy.prompt_guard.unwrap();
		let guard = &prompt_guard.request[0];

		assert_eq!(guard.rejection.status.as_u16(), 451);
		assert_eq!(
			guard.rejection.body.as_ref(),
			b"Content blocked by security policy"
		);
	}
}

#[test]
fn test_bedrock_guardrails_user_credentials_take_precedence() {
	use secrecy::SecretString;

	use crate::http::auth::{AwsAuth, BackendAuth, BackendAuthKind};
	use crate::store::BindStore;
	use crate::types::agent::BackendTrafficPolicy;

	let guardrails = BedrockGuardrails {
		guardrail_identifier: strng::new("test-guardrail"),
		guardrail_version: strng::new("1"),
		region: strng::new("us-east-1"),
		policies: vec![BackendTrafficPolicy::backend_auth(BackendAuthKind::Aws(
			AwsAuth::ExplicitConfig {
				access_key_id: SecretString::new("AKIAIOSFODNN7EXAMPLE".into()),
				secret_access_key: SecretString::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into()),
				region: Some("us-east-1".to_string()),
				session_token: None,
				service_name: None,
			},
		))],
	};

	let pols = guardrails.build_request_policies();

	// Resolve through the real policy resolution code (same path as call_with_explicit_policies)
	let store = BindStore::default();
	let resolved = store.inline_backend_policies(&pols);

	assert!(
		matches!(
			resolved.backend_auth,
			Some(BackendAuth {
				kind: Some(BackendAuthKind::Aws(AwsAuth::ExplicitConfig { .. })),
				..
			})
		),
		"Expected user-provided explicit AWS credentials to take precedence over \
		 the implicit fallback, but got: {:?}",
		resolved.backend_auth
	);
}

#[test]
fn test_bedrock_guardrails_api_key_auth_takes_precedence() {
	use secrecy::SecretString;

	use crate::http::auth::{BackendAuth, BackendAuthKind};
	use crate::store::BindStore;
	use crate::types::agent::BackendTrafficPolicy;

	let guardrails = BedrockGuardrails {
		guardrail_identifier: strng::new("test-guardrail"),
		guardrail_version: strng::new("1"),
		region: strng::new("us-east-1"),
		policies: vec![BackendTrafficPolicy::backend_auth(BackendAuthKind::Key {
			value: SecretString::new("bedrock-api-key".into()),
			location: None,
		})],
	};

	let pols = guardrails.build_request_policies();

	let store = BindStore::default();
	let resolved = store.inline_backend_policies(&pols);

	assert!(
		matches!(
			resolved.backend_auth,
			Some(BackendAuth {
				kind: Some(BackendAuthKind::Key {
					value: _,
					location: _
				}),
				..
			})
		),
		"Expected Bedrock API key auth to take precedence over \
		 the implicit AWS fallback, but got: {:?}",
		resolved.backend_auth
	);
}

#[test]
fn test_bedrock_guardrails_implicit_auth_used_when_no_user_credentials() {
	use crate::http::auth::{AwsAuth, BackendAuth, BackendAuthKind};
	use crate::store::BindStore;

	let guardrails = BedrockGuardrails {
		guardrail_identifier: strng::new("test-guardrail"),
		guardrail_version: strng::new("1"),
		region: strng::new("us-west-2"),
		policies: vec![],
	};

	let pols = guardrails.build_request_policies();

	let store = BindStore::default();
	let resolved = store.inline_backend_policies(&pols);

	assert!(
		matches!(
			resolved.backend_auth,
			Some(BackendAuth {
				kind: Some(BackendAuthKind::Aws(AwsAuth::Implicit {
					service_name: None,
					assume_role: None,
					..
				})),
				..
			})
		),
		"Expected implicit AWS auth when no user credentials are provided, but got: {:?}",
		resolved.backend_auth
	);
}

#[test]
fn test_google_model_armor_user_credentials_take_precedence() {
	use secrecy::SecretString;

	use crate::http::auth::{BackendAuth, BackendAuthKind};
	use crate::store::BindStore;
	use crate::types::agent::BackendTrafficPolicy;

	let model_armor = GoogleModelArmor {
		template_id: strng::new("test-template"),
		project_id: strng::new("test-project"),
		location: Some(strng::new("us-central1")),
		policies: vec![BackendTrafficPolicy::backend_auth(BackendAuthKind::Key {
			value: SecretString::new("user-provided-api-key".into()),
			location: None,
		})],
	};

	let pols = model_armor.build_request_policies();

	let store = BindStore::default();
	let resolved = store.inline_backend_policies(&pols);

	assert!(
		matches!(
			resolved.backend_auth,
			Some(BackendAuth {
				kind: Some(BackendAuthKind::Key {
					value: _,
					location: _
				}),
				..
			})
		),
		"Expected user-provided Key auth to take precedence over \
		 the implicit GCP fallback, but got: {:?}",
		resolved.backend_auth
	);
}

#[test]
fn test_google_model_armor_implicit_auth_used_when_no_user_credentials() {
	use crate::http::auth::{BackendAuth, BackendAuthKind};
	use crate::store::BindStore;

	let model_armor = GoogleModelArmor {
		template_id: strng::new("test-template"),
		project_id: strng::new("test-project"),
		location: None,
		policies: vec![],
	};

	let pols = model_armor.build_request_policies();

	let store = BindStore::default();
	let resolved = store.inline_backend_policies(&pols);

	assert!(
		matches!(
			resolved.backend_auth,
			Some(BackendAuth {
				kind: Some(BackendAuthKind::Gcp(_)),
				..
			})
		),
		"Expected implicit GCP auth when no user credentials are provided, but got: {:?}",
		resolved.backend_auth
	);
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum ChatFmt {
	Anthropic,
	Completions,
	Responses,
}

#[cfg(test)]
enum Expect {
	Masked(serde_json::Value),
	Rejected,
	Unchanged,
}

#[cfg(test)]
fn email_and_ssn() -> Vec<RegexRule> {
	vec![
		RegexRule::Builtin {
			builtin: Builtin::Email,
		},
		RegexRule::Builtin {
			builtin: Builtin::Ssn,
		},
	]
}

#[cfg(test)]
fn ssn_only() -> Vec<RegexRule> {
	vec![RegexRule::Builtin {
		builtin: Builtin::Ssn,
	}]
}

#[cfg(test)]
fn run_apply_regex(
	fmt: ChatFmt,
	action: Action,
	rules: Vec<RegexRule>,
	input: serde_json::Value,
) -> (GuardrailOutcome, serde_json::Value) {
	let rules = RegexRules { action, rules };
	let rejection = RequestRejection::default();
	match fmt {
		ChatFmt::Anthropic => {
			let mut req: crate::llm::types::messages::Request = serde_json::from_value(input).unwrap();
			let outcome = Policy::apply_regex(&mut req, &rules, &rejection).unwrap();
			(outcome, serde_json::to_value(&req).unwrap())
		},
		ChatFmt::Completions => {
			let mut req: crate::llm::types::completions::Request = serde_json::from_value(input).unwrap();
			let outcome = Policy::apply_regex(&mut req, &rules, &rejection).unwrap();
			(outcome, serde_json::to_value(&req).unwrap())
		},
		ChatFmt::Responses => {
			let mut req: crate::llm::types::responses::Request = serde_json::from_value(input).unwrap();
			let outcome = Policy::apply_regex(&mut req, &rules, &rejection).unwrap();
			(outcome, serde_json::to_value(&req).unwrap())
		},
	}
}

/// Same as `run_apply_regex`, but for `Policy::apply_regex_response`.
#[cfg(test)]
fn run_apply_regex_response(
	fmt: ChatFmt,
	action: Action,
	rules: Vec<RegexRule>,
	input: serde_json::Value,
) -> (GuardrailOutcome, serde_json::Value) {
	let rules = RegexRules { action, rules };
	let rejection = RequestRejection::default();
	match fmt {
		ChatFmt::Anthropic => {
			let mut resp: crate::llm::types::messages::Response = serde_json::from_value(input).unwrap();
			let outcome = Policy::apply_regex_response(&mut resp, &rules, &rejection).unwrap();
			(outcome, serde_json::to_value(&resp).unwrap())
		},
		ChatFmt::Completions => {
			let mut resp: crate::llm::types::completions::Response =
				serde_json::from_value(input).unwrap();
			let outcome = Policy::apply_regex_response(&mut resp, &rules, &rejection).unwrap();
			(outcome, serde_json::to_value(&resp).unwrap())
		},
		ChatFmt::Responses => {
			let mut resp: crate::llm::types::responses::Response = serde_json::from_value(input).unwrap();
			let outcome = Policy::apply_regex_response(&mut resp, &rules, &rejection).unwrap();
			(outcome, serde_json::to_value(&resp).unwrap())
		},
	}
}

#[cfg(test)]
#[rstest::rstest]
#[case::anthropic_mask_tool_output_untouched(
	ChatFmt::Anthropic,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"system": "You are a helpful assistant. Contact admin@example.com for help.",
		"messages": [
			{"role": "user", "content": "list pods"},
			{"role": "assistant", "content": [
				{"type": "text", "text": "Calling the tool."},
				{"type": "tool_use", "id": "toolu_01", "name": "k8s_get_resources", "input": {"ns": "kagent"}}
			]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_01", "content": "pod-a Running, owner ssn 123-45-6789"}
			]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_02", "content": [
					{"type": "text", "text": "reach ops@example.com"},
					{"type": "image", "source": {"type": "base64", "data": "aGk="}}
				]}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"system": "You are a helpful assistant. Contact <EMAIL_ADDRESS> for help.",
		"messages": [
			{"role": "user", "content": "list pods"},
			{"role": "assistant", "content": [
				{"type": "text", "text": "Calling the tool."},
				{"type": "tool_use", "id": "toolu_01", "name": "k8s_get_resources", "input": {"ns": "kagent"}}
			]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_01", "content": "pod-a Running, owner ssn 123-45-6789"}
			]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_02", "content": [
					{"type": "text", "text": "reach ops@example.com"},
					{"type": "image", "source": {"type": "base64", "data": "aGk="}}
				]}
			]}
		]
	}))
)]
#[case::anthropic_reject_matches_regular_message(
	ChatFmt::Anthropic,
	Action::Reject,
	ssn_only(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": "my ssn is 123-45-6789"}
		]
	}),
	Expect::Rejected
)]
#[case::anthropic_reject_does_not_reach_tool_output(
	ChatFmt::Anthropic,
	Action::Reject,
	ssn_only(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_01", "content": [
					{"type": "text", "text": "owner ssn 123-45-6789"}
				]}
			]}
		]
	}),
	Expect::Unchanged
)]
#[case::completions_mask_tool_output_untouched(
	ChatFmt::Completions,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": "email admin@example.com about the pods"},
			{"role": "assistant", "content": null, "tool_calls": [
				{"id": "call_01", "type": "function", "function": {"name": "list_pods", "arguments": "{}"}}
			]},
			{"role": "tool", "tool_call_id": "call_01", "content": "pod-a Running"},
			{"role": "tool", "tool_call_id": "call_02", "content": [
				{"type": "text", "text": "owner ssn 123-45-6789"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": "email <EMAIL_ADDRESS> about the pods"},
			{"role": "assistant", "tool_calls": [
				{"id": "call_01", "type": "function", "function": {"name": "list_pods", "arguments": "{}"}}
			]},
			{"role": "tool", "tool_call_id": "call_01", "content": "pod-a Running"},
			{"role": "tool", "tool_call_id": "call_02", "content": [
				{"type": "text", "text": "owner ssn 123-45-6789"}
			]}
		]
	}))
)]
#[case::completions_reject_does_not_reach_tool_output(
	ChatFmt::Completions,
	Action::Reject,
	ssn_only(),
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "tool", "tool_call_id": "call_01", "content": "owner ssn 123-45-6789"}
		]
	}),
	Expect::Unchanged
)]
#[case::responses_mask_tool_output_untouched(
	ChatFmt::Responses,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"role": "user", "content": [
				{"type": "input_text", "text": "email admin@example.com about the pods"}
			]},
			{"type": "function_call", "call_id": "call_01", "name": "list_pods", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "call_01", "output": "pod-a Running, owner ssn 123-45-6789"},
			{"type": "function_call_output", "call_id": "call_02", "output": [
				{"type": "output_text", "text": "billing contact admin@example.com"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"role": "user", "content": [
				{"type": "input_text", "text": "email <EMAIL_ADDRESS> about the pods"}
			]},
			{"type": "function_call", "call_id": "call_01", "name": "list_pods", "arguments": "{}"},
			{"type": "function_call_output", "call_id": "call_01", "output": "pod-a Running, owner ssn 123-45-6789"},
			{"type": "function_call_output", "call_id": "call_02", "output": [
				{"type": "output_text", "text": "billing contact admin@example.com"}
			]}
		]
	}))
)]
#[case::responses_reject_does_not_reach_tool_output(
	ChatFmt::Responses,
	Action::Reject,
	ssn_only(),
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "function_call_output", "call_id": "call_01", "output": "owner ssn 123-45-6789"}
		]
	}),
	Expect::Unchanged
)]
fn test_apply_regex_preserves_tool_structure(
	#[case] fmt: ChatFmt,
	#[case] action: Action,
	#[case] rules: Vec<RegexRule>,
	#[case] input: serde_json::Value,
	#[case] expected: Expect,
) {
	let (outcome, actual) = run_apply_regex(fmt, action, rules, input);
	match expected {
		Expect::Masked(expected) => {
			assert!(matches!(outcome, GuardrailOutcome::Masked));
			assert_eq!(actual, expected);
		},
		Expect::Rejected => assert!(matches!(outcome, GuardrailOutcome::Rejected(_))),
		Expect::Unchanged => assert!(matches!(outcome, GuardrailOutcome::None)),
	}
}

#[cfg(test)]
#[rstest::rstest]
#[case::anthropic_mask(
	ChatFmt::Anthropic,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"id": "msg_01", "type": "message", "role": "assistant", "model": "claude-sonnet-5",
		"stop_reason": "tool_use", "stop_sequence": null,
		"usage": {"input_tokens": 10, "output_tokens": 20},
		"content": [
			{"type": "text", "text": "Looking up the patient, ssn 123-45-6789 on file."},
			{"type": "tool_use", "id": "toolu_01", "name": "lookup_patient", "input": {"name": "John Doe"}}
		]
	}),
	Some(serde_json::json!({
		"id": "msg_01", "type": "message", "role": "assistant", "model": "claude-sonnet-5",
		"stop_reason": "tool_use", "stop_sequence": null,
		"usage": {"input_tokens": 10, "output_tokens": 20},
		"content": [
			{"type": "text", "text": "Looking up the patient, ssn <SSN> on file."},
			{"type": "tool_use", "id": "toolu_01", "name": "lookup_patient", "input": {"name": "John Doe"}}
		]
	}))
)]
#[case::anthropic_reject(
	ChatFmt::Anthropic,
	Action::Reject,
	ssn_only(),
	serde_json::json!({
		"id": "msg_01", "type": "message", "role": "assistant", "model": "claude-sonnet-5",
		"stop_reason": "tool_use", "stop_sequence": null,
		"usage": {"input_tokens": 10, "output_tokens": 20},
		"content": [
			{"type": "text", "text": "ssn 123-45-6789"},
			{"type": "tool_use", "id": "toolu_01", "name": "lookup_patient", "input": {"name": "John Doe"}}
		]
	}),
	None
)]
#[case::completions_mask(
	ChatFmt::Completions,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "gpt-4o",
		"usage": null,
		"choices": [
			{"message": {"role": "assistant", "content": "ssn 123-45-6789"}},
			{"message": {"role": "assistant", "content": null, "tool_calls": [
				{"id": "call_01", "type": "function", "function": {"name": "list_pods", "arguments": "{}"}}
			]}}
		]
	}),
	Some(serde_json::json!({
		"model": "gpt-4o",
		"usage": null,
		"choices": [
			{"message": {"role": "assistant", "content": "ssn <SSN>"}},
			{"message": {"role": "assistant", "tool_calls": [
				{"id": "call_01", "type": "function", "function": {"name": "list_pods", "arguments": "{}"}}
			]}}
		]
	}))
)]
#[case::responses_mask(
	ChatFmt::Responses,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"id": "resp_01", "status": "completed", "model": "gpt-4o",
		"output": [
			{"type": "message", "id": "msg_01", "role": "assistant", "status": "completed", "content": [
				{"type": "output_text", "annotations": [], "logprobs": null, "text": "Intro, all good."},
				{"type": "output_text", "annotations": [], "logprobs": null, "text": "ssn 123-45-6789"}
			]},
			{"type": "function_call", "arguments": "{}", "call_id": "call_01", "name": "list_pods"}
		]
	}),
	Some(serde_json::json!({
		"id": "resp_01", "status": "completed", "model": "gpt-4o",
		"output": [
			{"type": "message", "id": "msg_01", "role": "assistant", "status": "completed", "content": [
				{"type": "output_text", "annotations": [], "logprobs": null, "text": "Intro, all good."},
				{"type": "output_text", "annotations": [], "logprobs": null, "text": "ssn <SSN>"}
			]},
			{"type": "function_call", "arguments": "{}", "call_id": "call_01", "name": "list_pods"}
		]
	}))
)]
fn test_apply_regex_response_preserves_tool_structure(
	#[case] fmt: ChatFmt,
	#[case] action: Action,
	#[case] rules: Vec<RegexRule>,
	#[case] input: serde_json::Value,
	#[case] expected: Option<serde_json::Value>,
) {
	let (outcome, actual) = run_apply_regex_response(fmt, action, rules, input);
	match expected {
		Some(expected) => {
			assert!(matches!(outcome, GuardrailOutcome::Masked));
			assert_eq!(actual, expected);
		},
		None => assert!(matches!(outcome, GuardrailOutcome::Rejected(_))),
	}
}
