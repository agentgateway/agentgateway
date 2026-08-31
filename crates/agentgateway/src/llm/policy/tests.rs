use ::http::{HeaderName, HeaderValue};

use super::*;
use crate::telemetry::metrics::{GuardrailAction, GuardrailLabels, GuardrailPhase};
use crate::types::agent::HeaderValueMatch;

/// When a webhook guard fails open, exactly one metric must be emitted (`FailOpen`); the caller
/// must not additionally record `Allow`.
#[tokio::test]
async fn webhook_fail_open_emits_single_metric() {
	use crate::types::agent::SimpleBackendReference;

	let guard = PromptGuard {
		streaming: Default::default(),
		request: vec![RequestGuard {
			rejection: Default::default(),
			scope: default_content_scope(),
			kind: RequestGuardKind::Webhook(Webhook {
				target: SimpleBackendReference::Invalid,
				headers: Default::default(),
				forward_header_matches: vec![],
				failure_mode: FailureMode::FailOpen,
				action: RejectAuditAction::Reject,
			}),
		}],
		response: vec![],
	};

	let client = crate::test_helpers::policy_client();
	let log = GuardrailLog::default();
	let blocked = guard
		.apply_realtime_request_guards("hello world", &client, None, Some(&log))
		.await;
	assert!(blocked.is_none(), "FailOpen must not block the request");
	let entry = &log.take().unwrap()[0];
	assert_eq!(entry.guard, "webhook");
	assert_eq!(entry.action, "failOpen");

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

fn guardrail_metric(
	client: &crate::proxy::httpproxy::PolicyClient,
	phase: GuardrailPhase,
	action: GuardrailAction,
) -> u64 {
	client
		.inputs
		.metrics
		.guardrail_checks
		.get_or_create(&GuardrailLabels { phase, action })
		.get()
}

#[tokio::test]
async fn audit_mode_records_allow_when_nothing_matches() {
	let guard = ResponseGuard {
		rejection: Default::default(),
		kind: ResponseGuardKind::Regex(RegexRules {
			action: Action::Audit,
			rules: vec![RegexRule::Regex {
				pattern: regex::Regex::new("SECRET").unwrap(),
			}],
		}),
	};
	let client = crate::test_helpers::policy_client();
	let mut resp = TextResponse {
		content: "nothing sensitive here".to_string(),
	};
	let headers = ::http::HeaderMap::new();
	let (action, rejection) =
		Policy::apply_single_response_guard(&guard, &mut resp, &headers, &client, None, None, true)
			.await
			.unwrap();
	assert!(rejection.is_none(), "audit mode must never reject");
	Policy::record_guardrail_trip(&client, GuardrailPhase::Response, action);

	assert_eq!(
		guardrail_metric(&client, GuardrailPhase::Response, GuardrailAction::Allow),
		1,
		"a non-matching audit guard records Allow"
	);
	assert_eq!(
		guardrail_metric(&client, GuardrailPhase::Response, GuardrailAction::Audit),
		0,
		"Audit must not be recorded when the guard would not have enforced"
	);
}

#[tokio::test]
async fn audit_mode_records_audit_and_passes_through_on_match() {
	let guard = ResponseGuard {
		rejection: Default::default(),
		kind: ResponseGuardKind::Regex(RegexRules {
			action: Action::Audit,
			rules: vec![RegexRule::Regex {
				pattern: regex::Regex::new("SECRET").unwrap(),
			}],
		}),
	};
	let client = crate::test_helpers::policy_client();
	let original = "my SECRET token".to_string();
	let mut resp = TextResponse {
		content: original.clone(),
	};
	let headers = ::http::HeaderMap::new();
	let (action, rejection) =
		Policy::apply_single_response_guard(&guard, &mut resp, &headers, &client, None, None, true)
			.await
			.unwrap();
	assert_eq!(
		action,
		GuardrailAction::Audit,
		"a matching audit guard yields Audit, not Reject/Mask"
	);
	assert!(rejection.is_none(), "audit mode must never reject");
	Policy::record_guardrail_trip(&client, GuardrailPhase::Response, action);
	assert_eq!(resp.content, original, "audit mode must not mutate content");

	assert_eq!(
		guardrail_metric(&client, GuardrailPhase::Response, GuardrailAction::Audit),
		1
	);
	assert_eq!(
		guardrail_metric(&client, GuardrailPhase::Response, GuardrailAction::Reject),
		0
	);
}

#[test]
fn bedrock_audit_outcome_maps_interventions_to_audit() {
	use serde_json::json;
	let outcome =
		|v: serde_json::Value| -> (GuardrailOutcome<RequestGuardMutation>, Option<GuardDetail>) {
			let resp: bedrock_guardrails::ApplyGuardrailResponse = serde_json::from_value(v).unwrap();
			Policy::bedrock_audit_outcome(resp, &bedrock_test_config())
		};

	let (blocked, detail) = outcome(json!({
		"action": "GUARDRAIL_INTERVENED",
		"assessments": [{ "contentPolicy": { "filters": [{ "action": "BLOCKED", "type": "HATE" }] } }]
	}));
	assert!(
		matches!(blocked, GuardrailOutcome::Audit),
		"a would-block assessment audits, never rejects"
	);
	assert_eq!(
		detail.unwrap().assessments[0]["contentPolicy"]["filters"][0]["type"],
		"HATE"
	);

	let (anonymized, _) = outcome(json!({
		"action": "GUARDRAIL_INTERVENED",
		"outputs": [{"text": "redacted {NAME}"}],
		"assessments": [{ "sensitiveInformationPolicy": { "piiEntities": [{ "action": "ANONYMIZED", "type": "NAME" }] } }]
	}));
	assert!(
		matches!(anonymized, GuardrailOutcome::Audit),
		"a would-mask assessment audits, never masks"
	);

	let (detect_only, detail) = outcome(json!({
		"action": "NONE",
		"assessments": [{ "contentPolicy": {
			"filters": [{ "action": "NONE", "confidence": "LOW", "detected": true, "type": "VIOLENCE" }]
		} }]
	}));
	assert!(matches!(detect_only, GuardrailOutcome::None));
	assert!(detail.is_none());

	let (benign, _) = outcome(json!({ "action": "NONE", "assessments": [{}] }));
	assert!(matches!(benign, GuardrailOutcome::None));
}

#[tokio::test]
async fn streaming_guard_records_one_metric_per_stream() {
	use crate::llm::policy::streaming_guardrails::make_evaluator;

	let guard = ResponseGuard {
		rejection: Default::default(),
		kind: ResponseGuardKind::Regex(RegexRules {
			action: Action::Audit,
			rules: vec![RegexRule::Regex {
				pattern: regex::Regex::new("SECRET").unwrap(),
			}],
		}),
	};
	let client = crate::test_helpers::policy_client();
	let headers = ::http::HeaderMap::new();
	let mut evaluator = make_evaluator(&guard, client.clone(), headers, None, Default::default());

	let _ = evaluator.evaluate("clean window one").await.unwrap();
	let _ = evaluator.evaluate("this window has SECRET").await.unwrap();
	let _ = evaluator.evaluate("clean window three").await.unwrap();
	assert_eq!(
		guardrail_metric(&client, GuardrailPhase::Response, GuardrailAction::Audit),
		0,
		"no metric recorded mid-stream"
	);

	drop(evaluator);
	assert_eq!(
		guardrail_metric(&client, GuardrailPhase::Response, GuardrailAction::Audit),
		1,
		"exactly one Audit recorded for the whole stream, not one per window"
	);
	assert_eq!(
		guardrail_metric(&client, GuardrailPhase::Response, GuardrailAction::Allow),
		0,
		"the matching window's Audit outranks the benign windows' Allow"
	);
}

#[test]
fn azure_blocked_records_guardrail_info_without_matched_text() {
	let resp: azure_content_safety::AnalyzeTextResponse = serde_json::from_value(serde_json::json!({
		"blocklistsMatch": [{
			"blocklistName": "banned-terms",
			"blocklistItemId": "item-1",
			"blocklistItemText": "secretword"
		}],
		"categoriesAnalysis": [
			{"category": "Violence", "severity": 4},
			{"category": "Hate", "severity": 0}
		]
	}))
	.unwrap();
	let detail = Policy::azure_analyze_detail(&resp, 2);
	let assessment = &detail.assessments[0];
	assert_eq!(
		assessment["categoriesAnalysis"],
		serde_json::json!([{"category": "Violence", "severity": 4}])
	);
	assert_eq!(
		assessment["blocklistMatches"],
		serde_json::json!(["banned-terms"])
	);
	assert!(
		!serde_json::to_string(assessment)
			.unwrap()
			.contains("secretword")
	);
}

#[test]
fn model_armor_blocked_records_guardrail_info() {
	let resp: google_model_armor::SanitizeResponse = serde_json::from_value(serde_json::json!({
		"sanitizationResult": {
			"filterResults": [
				{"raiFilterResult": {"matchState": "MATCH_FOUND"}},
				{"piAndJailbreakFilterResult": {"matchState": "MATCH_FOUND"}}
			]
		}
	}))
	.unwrap();
	let config = GoogleModelArmor {
		template_id: strng::new("templates/my-template"),
		project_id: strng::new("proj"),
		location: None,
		policies: vec![],
		action: Default::default(),
	};
	let (outcome, detail): (GuardrailOutcome<RequestGuardMutation>, _) =
		Policy::model_armor_outcome(resp, &config, &RequestRejection::default());
	assert!(matches!(outcome, GuardrailOutcome::Rejected(_)));
	let detail = detail.unwrap();
	assert_eq!(
		detail.guardrail_id.as_deref(),
		Some("templates/my-template")
	);
	assert_eq!(
		detail.assessments[0]["matchedFilters"],
		serde_json::json!(["raiFilterResult", "piAndJailbreakFilterResult"])
	);
}

#[test]
fn moderation_flagged_records_guardrail_info() {
	let cats = [
		"hate",
		"hate/threatening",
		"harassment",
		"harassment/threatening",
		"illicit",
		"illicit/violent",
		"self-harm",
		"self-harm/intent",
		"self-harm/instructions",
		"sexual",
		"sexual/minors",
		"violence",
		"violence/graphic",
	];
	let obj = |v: fn(&str) -> serde_json::Value| -> serde_json::Value {
		cats.iter().map(|c| (c.to_string(), v(c))).collect()
	};
	let resp: async_openai::types::moderations::CreateModerationResponse =
		serde_json::from_value(serde_json::json!({
			"id": "modr-1",
			"model": "omni-moderation-latest",
			"results": [{
				"flagged": true,
				"categories": obj(|c| serde_json::json!(c == "violence")),
				"category_scores": obj(|_| serde_json::json!(0.5)),
				"category_applied_input_types": obj(|_| serde_json::json!(["text"])),
			}]
		}))
		.unwrap();

	let (outcome, detail) = Policy::moderation_outcome(resp, &RequestRejection::default(), false);
	assert!(matches!(outcome, GuardrailOutcome::Rejected(_)));
	assert_eq!(
		detail.unwrap().assessments[0]["flaggedCategories"],
		serde_json::json!(["violence"])
	);
}

#[test]
fn webhook_reject_records_guardrail_info() {
	use crate::llm::policy::webhook::{RejectAction, RequestAction};

	let (outcome, detail) = Policy::webhook_request_outcome(RequestAction::Reject(RejectAction {
		body: "blocked".to_string(),
		status_code: 403,
		reason: Some("policy violation".to_string()),
	}))
	.unwrap();
	assert!(matches!(outcome, GuardrailOutcome::Rejected(_)));
	assert_eq!(
		detail.unwrap().action_reason.as_deref(),
		Some("policy violation")
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
			response.into_output_texts(),
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
			response.into_output_texts(),
			vec!["First message with {NAME}", "Second message with {EMAIL}"]
		);
	}

	#[test]
	fn test_apply_guardrail_response_intervened_no_assessments() {
		let json = json!({
			"action": "GUARDRAIL_INTERVENED",
			"outputs": [{"text": "I can't help with that."}],
			"assessments": [{
				"automatedReasoningPolicy": {
					"findings": [{"invalid": {"logicWarning": {"type": "ALWAYS_FALSE"}}}]
				}
			}]
		});
		let response: ApplyGuardrailResponse = serde_json::from_value(json).unwrap();
		assert!(!response.is_blocked());
		assert!(!response.is_anonymized());
		assert!(response.is_intervened());
	}
}

/// Build an intervened ApplyGuardrail response from output texts and assessments JSON.
fn bedrock_intervened(
	outputs: &[&str],
	assessments: serde_json::Value,
) -> bedrock_guardrails::ApplyGuardrailResponse {
	serde_json::from_value(serde_json::json!({
		"action": "GUARDRAIL_INTERVENED",
		"outputs": outputs.iter().map(|t| serde_json::json!({"text": t})).collect::<Vec<_>>(),
		"assessments": assessments,
	}))
	.unwrap()
}

fn bedrock_anonymized_assessments() -> serde_json::Value {
	serde_json::json!([{
		"sensitiveInformationPolicy": {
			"piiEntities": [{"action": "ANONYMIZED", "match": "x", "type": "NAME"}]
		}
	}])
}

fn bedrock_test_config() -> BedrockGuardrails {
	BedrockGuardrails {
		guardrail_identifier: strng::new("gr-test"),
		guardrail_version: strng::new("1"),
		region: strng::new("us-west-2"),
		policies: vec![],
		action: Default::default(),
	}
}

/// Assert what the Bedrock guard would send, then run an anonymize verdict through
/// the real outcome path and apply the resulting mask to the request.
fn apply_bedrock_request_mask(req: &mut dyn RequestType, sent: &[&str], masked: &[&str]) {
	assert_eq!(Policy::request_texts(req), sent);
	let (outcome, _) = Policy::bedrock_guardrail_outcome(
		bedrock_intervened(masked, bedrock_anonymized_assessments()),
		sent.len(),
		&RequestRejection::default(),
		&bedrock_test_config(),
	);
	assert!(matches!(outcome, GuardrailOutcome::Masked(_)));
	let (_, rejection) =
		Policy::apply_request_guard_outcome(outcome.map_mask(RequestGuardMutation::Texts), req)
			.unwrap();
	assert!(rejection.is_none());
}

/// Same as `apply_bedrock_request_mask`, but for responses.
fn apply_bedrock_response_mask(resp: &mut dyn ResponseType, sent: &[&str], masked: &[&str]) {
	assert_eq!(Policy::response_texts(resp), sent);
	let (outcome, _) = Policy::bedrock_guardrail_outcome(
		bedrock_intervened(masked, bedrock_anonymized_assessments()),
		sent.len(),
		&RequestRejection::default(),
		&bedrock_test_config(),
	);
	assert!(matches!(outcome, GuardrailOutcome::Masked(_)));
	let (_, rejection) =
		Policy::apply_response_guard_outcome(outcome.map_mask(ResponseGuardMutation::Texts), resp)
			.unwrap();
	assert!(rejection.is_none());
}

#[test]
fn bedrock_request_mask_preserves_completions_content_parts() {
	let mut req: crate::llm::types::completions::Request =
		serde_json::from_value(serde_json::json!({
			"model": "gpt-4o",
			"messages": [{
				"role": "user",
				"content": [
					{"type": "text", "text": "My name is Jane"},
					{"type": "image_url", "image_url": {"url": "https://example.com/image.png"}},
					{"type": "text", "text": "Email jane@example.com"}
				]
			}]
		}))
		.unwrap();

	apply_bedrock_request_mask(
		&mut req,
		&["My name is Jane", "Email jane@example.com"],
		&["My name is {NAME}", "Email {EMAIL}"],
	);

	let value = serde_json::to_value(req).unwrap();
	assert_eq!(
		value["messages"][0]["content"][0]["text"],
		"My name is {NAME}"
	);
	assert_eq!(
		value["messages"][0]["content"][1]["image_url"]["url"],
		"https://example.com/image.png"
	);
	assert_eq!(value["messages"][0]["content"][2]["text"], "Email {EMAIL}");
}

#[test]
fn bedrock_request_mask_preserves_anthropic_content_parts() {
	let mut req: crate::llm::types::messages::Request = serde_json::from_value(serde_json::json!({
		"model": "claude-sonnet-4-20250514",
		"max_tokens": 128,
		"system": [{"type": "text", "text": "Contact Jane", "cache_control": {"type": "ephemeral"}}],
		"messages": [{
			"role": "user",
			"content": [
				{"type": "text", "text": "Email jane@example.com"},
				{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AA=="}}
			]
		}]
	}))
	.unwrap();

	apply_bedrock_request_mask(
		&mut req,
		&["Contact Jane", "Email jane@example.com"],
		&["Contact {NAME}", "Email {EMAIL}"],
	);

	let value = serde_json::to_value(req).unwrap();
	assert_eq!(value["system"][0]["text"], "Contact {NAME}");
	assert_eq!(value["system"][0]["cache_control"]["type"], "ephemeral");
	assert_eq!(value["messages"][0]["content"][0]["text"], "Email {EMAIL}");
	assert_eq!(value["messages"][0]["content"][1]["source"]["data"], "AA==");
}

#[test]
fn bedrock_request_mask_preserves_responses_input_items() {
	let mut req: crate::llm::types::responses::Request = serde_json::from_value(serde_json::json!({
		"model": "gpt-4o",
		"instructions": "Contact Jane",
		"input": [{
			"role": "user",
			"content": [
				{"type": "input_text", "text": "Email jane@example.com"},
				{"type": "input_image", "image_url": "https://example.com/image.png"}
			]
		}]
	}))
	.unwrap();

	apply_bedrock_request_mask(
		&mut req,
		&["Contact Jane", "Email jane@example.com"],
		&["Contact {NAME}", "Email {EMAIL}"],
	);

	let value = serde_json::to_value(req).unwrap();
	assert_eq!(value["instructions"], "Contact {NAME}");
	assert_eq!(value["input"][0]["content"][0]["text"], "Email {EMAIL}");
	assert_eq!(
		value["input"][0]["content"][1]["image_url"],
		"https://example.com/image.png"
	);
}

/// A blocked intervention returns a single canned message no matter how many blocks
/// were sent; it must reject rather than error out or misapply the canned text.
#[test]
fn bedrock_blocked_intervention_with_canned_output_rejects() {
	let resp = bedrock_intervened(
		&["Sorry, I can't help with that."],
		serde_json::json!([{
			"topicPolicy": {"topics": [{"action": "BLOCKED", "name": "Finance", "type": "DENY"}]}
		}]),
	);
	let (outcome, _) = Policy::bedrock_guardrail_outcome(
		resp,
		3,
		&RequestRejection::default(),
		&bedrock_test_config(),
	);
	assert!(matches!(outcome, GuardrailOutcome::Rejected(_)));
}

/// Automated reasoning findings carry no `action` field; even when the output count
/// happens to match, the canned message must not be applied as a mask.
#[test]
fn bedrock_unrecognized_intervention_rejects_instead_of_masking() {
	let resp = bedrock_intervened(
		&["I can't help with that."],
		serde_json::json!([{
			"automatedReasoningPolicy": {"findings": [{"impossible": {}}]}
		}]),
	);
	let (outcome, _) = Policy::bedrock_guardrail_outcome(
		resp,
		1,
		&RequestRejection::default(),
		&bedrock_test_config(),
	);
	assert!(matches!(outcome, GuardrailOutcome::Rejected(_)));
}

/// Masked outputs that can't be mapped one-to-one onto the sent blocks must reject
/// rather than misalign replacements or fail the whole request.
#[test]
fn bedrock_masked_output_count_mismatch_rejects() {
	let resp = bedrock_intervened(&["Email {EMAIL}"], bedrock_anonymized_assessments());
	// The final decision is a rejection despite the anonymize verdict.
	let (outcome, detail) = Policy::bedrock_guardrail_outcome(
		resp,
		2,
		&RequestRejection::default(),
		&bedrock_test_config(),
	);
	assert!(matches!(outcome, GuardrailOutcome::Rejected(_)));
	assert!(detail.is_some());
}

/// A masked intervention must carry detail with its assessment metadata.
#[test]
fn bedrock_masked_intervention_records_guardrail_info() {
	let (outcome, detail) = Policy::bedrock_guardrail_outcome(
		bedrock_intervened(&["My name is {NAME}"], bedrock_anonymized_assessments()),
		1,
		&RequestRejection::default(),
		&bedrock_test_config(),
	);
	assert!(matches!(outcome, GuardrailOutcome::Masked(_)));
	let detail = detail.unwrap();
	assert_eq!(detail.guardrail_id.as_deref(), Some("gr-test"));
	assert_eq!(detail.guardrail_version.as_deref(), Some("1"));
	assert_eq!(
		detail.assessments[0]["sensitiveInformationPolicy"]["piiEntities"][0]["type"],
		"NAME"
	);
}

/// A blocked intervention must record a reject entry carrying the action reason
/// and the assessment metadata, without matched content or unknown fields.
#[test]
fn bedrock_blocked_intervention_records_guardrail_info() {
	let resp: bedrock_guardrails::ApplyGuardrailResponse =
		serde_json::from_value(serde_json::json!({
			"action": "GUARDRAIL_INTERVENED",
			"actionReason": "Guardrail blocked.",
			"outputs": [{"text": "Sorry, I can't help with that."}],
			"assessments": [{
				"topicPolicy": {"topics": [{"action": "BLOCKED", "name": "Finance", "type": "DENY"}]},
				"sensitiveInformationPolicy": {
					"piiEntities": [{"match": "john.doe@example.com", "type": "EMAIL", "action": "BLOCKED"}],
					"regexes": [{"name": "acct", "regex": "a-[0-9]+", "match": "a-42", "action": "BLOCKED"}]
				},
				"wordPolicy": {"customWords": [{"match": "secretword", "action": "BLOCKED"}]},
				"automatedReasoningPolicy": {"findings": [{"claim": "user text"}]},
				"invocationMetrics": {"guardrailProcessingLatency": 128},
				"appliedGuardrailDetails": {
					"guardrailId": "gr-test",
					"guardrailVersion": "1",
					"guardrailOrigin": ["REQUEST"]
				}
			}]
		}))
		.unwrap();
	let (outcome, detail) = Policy::bedrock_guardrail_outcome(
		resp,
		1,
		&RequestRejection::default(),
		&bedrock_test_config(),
	);
	assert!(matches!(outcome, GuardrailOutcome::Rejected(_)));
	let detail = detail.unwrap();
	assert_eq!(detail.action_reason.as_deref(), Some("Guardrail blocked."));
	let assessment = &detail.assessments[0];
	assert_eq!(assessment["topicPolicy"]["topics"][0]["name"], "Finance");
	assert_eq!(
		assessment["sensitiveInformationPolicy"]["piiEntities"][0]["type"],
		"EMAIL"
	);
	assert_eq!(
		assessment["invocationMetrics"]["guardrailProcessingLatency"],
		128
	);
	assert_eq!(
		assessment["appliedGuardrailDetails"]["guardrailId"],
		"gr-test"
	);
	// An unknown policy keeps only a top-level marker; nested content and keys
	// under it are gone, as are matched-text fields under known policies.
	assert_eq!(assessment["automatedReasoningPolicy"], "[redacted]");
	assert!(
		assessment["sensitiveInformationPolicy"]["piiEntities"][0]
			.get("match")
			.is_none()
	);
	let json = serde_json::to_string(&detail.assessments).unwrap();
	assert!(!json.contains("john.doe"));
	assert!(!json.contains("secretword"));
	assert!(!json.contains("a-42"));
	assert!(!json.contains("findings"));
}

#[test]
fn bedrock_default_scope_skips_tool_texts_and_keeps_mask_aligned() {
	let mut req: crate::llm::types::completions::Request =
		serde_json::from_value(serde_json::json!({
			"model": "gpt-4o",
			"messages": [
				{"role": "system", "content": "be helpful"},
				{"role": "tool", "tool_call_id": "call_1", "content": "customer jane@example.com"},
				{"role": "user", "content": "email jane@example.com please"}
			]
		}))
		.unwrap();

	let (sent, in_scope) = Policy::scoped_request_texts(&mut req, &default_content_scope());
	assert_eq!(sent, vec!["be helpful", "email jane@example.com please"]);
	assert_eq!(in_scope, vec![true, false, true]);

	let outcome = Policy::bedrock_guardrail_outcome(
		bedrock_intervened(
			&["be helpful", "email {EMAIL} please"],
			bedrock_anonymized_assessments(),
		),
		sent.len(),
		&RequestRejection::default(),
		&bedrock_test_config(),
	)
	.0
	.map_mask(|mask| RequestGuardMutation::Texts(mask.scatter(&in_scope)));
	let (_, rejection) = Policy::apply_request_guard_outcome(outcome, &mut req).unwrap();
	assert!(rejection.is_none());

	let value = serde_json::to_value(req).unwrap();
	assert_eq!(value["messages"][1]["content"], "customer jane@example.com");
	assert_eq!(value["messages"][2]["content"], "email {EMAIL} please");
}

#[test]
fn bedrock_response_mask_preserves_choice_metadata() {
	let mut resp: crate::llm::types::completions::Response =
		serde_json::from_value(serde_json::json!({
			"model": "gpt-4o",
			"usage": null,
			"choices": [{
				"index": 0,
				"finish_reason": "tool_calls",
				"message": {
					"role": "assistant",
					"content": "Contact Jane",
					"tool_calls": [{"id": "call_1", "type": "function"}]
				}
			}]
		}))
		.unwrap();

	apply_bedrock_response_mask(&mut resp, &["Contact Jane"], &["Contact {NAME}"]);

	let value = serde_json::to_value(resp).unwrap();
	assert_eq!(value["choices"][0]["message"]["content"], "Contact {NAME}");
	assert_eq!(
		value["choices"][0]["message"]["tool_calls"][0]["id"],
		"call_1"
	);
	assert_eq!(value["choices"][0]["finish_reason"], "tool_calls");
}

/// Masking rewrites the text a citation's offsets point into; stale offsets and
/// logprobs must be dropped, while untouched parts keep theirs.
#[test]
fn bedrock_response_mask_clears_stale_annotation_offsets() {
	let mut resp: crate::llm::types::responses::Response =
		serde_json::from_value(serde_json::json!({
			"id": "resp_01", "status": "completed", "model": "gpt-4o",
			"output": [
				{"type": "message", "id": "msg_01", "role": "assistant", "status": "completed", "content": [
					{"type": "output_text", "logprobs": null, "text": "See the docs.", "annotations": [
						{"type": "url_citation", "url": "https://example.com", "title": "Docs", "start_index": 0, "end_index": 12}
					]},
					{"type": "output_text", "logprobs": null, "text": "Email jane@example.com", "annotations": []}
				]}
			]
		}))
		.unwrap();

	apply_bedrock_response_mask(
		&mut resp,
		&["See the docs.", "Email jane@example.com"],
		&["See the docs.", "Email {EMAIL}"],
	);

	let value = serde_json::to_value(resp).unwrap();
	let parts = &value["output"][0]["content"];
	// untouched part keeps its citation
	assert_eq!(parts[0]["text"], "See the docs.");
	assert_eq!(parts[0]["annotations"][0]["url"], "https://example.com");
	assert_eq!(parts[1]["text"], "Email {EMAIL}");
	assert_eq!(parts[1]["annotations"], serde_json::json!([]));
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
	fn test_bedrock_action_defaults_to_reject() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"bedrockGuardrails": {
						"guardrailIdentifier": "gr",
						"guardrailVersion": "1",
						"region": "us-west-2"
					}
				}]
			}
		});
		let policy: Policy = serde_json::from_value(json).unwrap();
		match &policy.prompt_guard.unwrap().request[0].kind {
			RequestGuardKind::BedrockGuardrails(bg) => {
				assert_eq!(bg.action, RejectAuditAction::Reject);
			},
			_ => panic!("Expected BedrockGuardrails guard kind"),
		}
	}

	#[test]
	fn test_bedrock_action_audit_deserializes() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"bedrockGuardrails": {
						"guardrailIdentifier": "gr",
						"guardrailVersion": "1",
						"region": "us-west-2",
						"action": "audit"
					}
				}]
			}
		});
		let policy: Policy = serde_json::from_value(json).unwrap();
		match &policy.prompt_guard.unwrap().request[0].kind {
			RequestGuardKind::BedrockGuardrails(bg) => {
				assert_eq!(bg.action, RejectAuditAction::Audit);
			},
			_ => panic!("Expected BedrockGuardrails guard kind"),
		}
	}

	#[test]
	fn test_reject_only_kinds_accept_audit_action() {
		let json = json!({
			"promptGuard": {
				"request": [
					{ "openAIModeration": { "action": "audit" } },
					{ "googleModelArmor": { "templateId": "t", "projectId": "p", "action": "audit" } }
				]
			}
		});
		let policy: Policy = serde_json::from_value(json).unwrap();
		let request = &policy.prompt_guard.unwrap().request;
		match &request[0].kind {
			RequestGuardKind::OpenAIModeration(m) => {
				assert_eq!(m.action, RejectAuditAction::Audit);
			},
			_ => panic!("Expected OpenAIModeration guard kind"),
		}
		match &request[1].kind {
			RequestGuardKind::GoogleModelArmor(gma) => {
				assert_eq!(gma.action, RejectAuditAction::Audit);
			},
			_ => panic!("Expected GoogleModelArmor guard kind"),
		}
	}

	#[test]
	fn test_reject_only_kind_rejects_mask_action() {
		let json = json!({
			"promptGuard": {
				"request": [{ "openAIModeration": { "action": "mask" } }]
			}
		});
		assert!(
			serde_json::from_value::<Policy>(json).is_err(),
			"openAIModeration must reject action=mask"
		);
	}

	#[test]
	fn test_regex_action_audit_deserializes() {
		let json = json!({
			"promptGuard": {
				"request": [{
					"regex": { "action": "audit", "rules": [{ "pattern": "secret" }] }
				}]
			}
		});
		let policy: Policy = serde_json::from_value(json).unwrap();
		match &policy.prompt_guard.unwrap().request[0].kind {
			RequestGuardKind::Regex(rr) => {
				assert!(matches!(rr.action, Action::Audit));
			},
			_ => panic!("Expected Regex guard kind"),
		}
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
		action: RejectAuditAction::Reject,
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
		action: RejectAuditAction::Reject,
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
		action: RejectAuditAction::Reject,
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
		action: RejectAuditAction::Reject,
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
		action: RejectAuditAction::Reject,
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
	Gemini,
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

/// Opted in to also scanning tool call inputs and results.
#[cfg(test)]
fn all_scopes() -> Vec<ContentScope> {
	vec![
		ContentScope::SystemPrompt,
		ContentScope::Messages,
		ContentScope::ToolOutput,
		ContentScope::ToolInput,
	]
}

#[tokio::test]
async fn regex_reject_records_guardrail_info() {
	let mut req: crate::llm::types::completions::Request =
		serde_json::from_value(serde_json::json!({
			"model": "gpt-4o",
			"messages": [{"role": "user", "content": "my ssn is 123-45-6789"}]
		}))
		.unwrap();
	let guard = RequestGuard {
		rejection: Default::default(),
		scope: default_content_scope(),
		kind: RequestGuardKind::Regex(RegexRules {
			action: Action::Reject,
			rules: ssn_only(),
		}),
	};
	let client = crate::test_helpers::policy_client();
	let log = GuardrailLog::default();
	let (_, rejection) = Policy::apply_single_request_guard(
		&guard,
		&mut req,
		&::http::HeaderMap::new(),
		&client,
		None,
		None,
		Some(&log),
	)
	.await
	.unwrap();
	assert!(rejection.is_some());
	let entry = &log.take().unwrap()[0];
	assert_eq!(entry.phase, "request");
	assert_eq!(entry.guard, "regex");
	assert_eq!(entry.action, "reject");
}

#[test]
fn regex_evaluation_does_not_mutate_until_enforced() {
	let mut req: crate::llm::types::completions::Request =
		serde_json::from_value(serde_json::json!({
			"model": "gpt-4o",
			"messages": [{"role": "user", "content": "my ssn is 123-45-6789"}]
		}))
		.unwrap();
	let before = serde_json::to_value(&req).unwrap();
	let rules = RegexRules {
		action: Action::Mask,
		rules: ssn_only(),
	};

	let rejection = RequestRejection::default();
	let result =
		Policy::evaluate_regex_request(&mut req, &rules, &rejection, &default_content_scope());
	assert!(matches!(&result, GuardrailOutcome::Masked(_)));
	assert_eq!(serde_json::to_value(&req).unwrap(), before);

	let (action, rejection) = Policy::apply_request_guard_outcome(result, &mut req).unwrap();
	assert_eq!(action, GuardrailAction::Mask);
	assert!(rejection.is_none());
	assert_ne!(serde_json::to_value(&req).unwrap(), before);
}

#[cfg(test)]
fn run_apply_regex(
	fmt: ChatFmt,
	action: Action,
	rules: Vec<RegexRule>,
	input: serde_json::Value,
) -> (GuardrailAction, serde_json::Value) {
	run_apply_regex_scoped(fmt, action, rules, default_content_scope(), input)
}

/// Same as `run_apply_regex`, but with an explicit guard scope.
#[cfg(test)]
fn run_apply_regex_scoped(
	fmt: ChatFmt,
	action: Action,
	rules: Vec<RegexRule>,
	scope: Vec<ContentScope>,
	input: serde_json::Value,
) -> (GuardrailAction, serde_json::Value) {
	let rules = RegexRules { action, rules };
	let rejection = RequestRejection::default();
	match fmt {
		ChatFmt::Anthropic => {
			let mut req: crate::llm::types::messages::Request = serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex(&mut req, &rules, &rejection, &scope).unwrap();
			(action, serde_json::to_value(&req).unwrap())
		},
		ChatFmt::Completions => {
			let mut req: crate::llm::types::completions::Request = serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex(&mut req, &rules, &rejection, &scope).unwrap();
			(action, serde_json::to_value(&req).unwrap())
		},
		ChatFmt::Responses => {
			let mut req: crate::llm::types::responses::Request = serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex(&mut req, &rules, &rejection, &scope).unwrap();
			(action, serde_json::to_value(&req).unwrap())
		},
		ChatFmt::Gemini => {
			let mut req: crate::llm::types::gemini::Request = serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex(&mut req, &rules, &rejection, &scope).unwrap();
			(action, req.to_value().unwrap())
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
) -> (GuardrailAction, serde_json::Value) {
	let rules = RegexRules { action, rules };
	let rejection = RequestRejection::default();
	match fmt {
		ChatFmt::Anthropic => {
			let mut resp: crate::llm::types::messages::Response = serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex_response(&mut resp, &rules, &rejection).unwrap();
			(action, serde_json::to_value(&resp).unwrap())
		},
		ChatFmt::Completions => {
			let mut resp: crate::llm::types::completions::Response =
				serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex_response(&mut resp, &rules, &rejection).unwrap();
			(action, serde_json::to_value(&resp).unwrap())
		},
		ChatFmt::Responses => {
			let mut resp: crate::llm::types::responses::Response = serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex_response(&mut resp, &rules, &rejection).unwrap();
			(action, serde_json::to_value(&resp).unwrap())
		},
		ChatFmt::Gemini => {
			let mut resp: crate::llm::types::gemini::Response = serde_json::from_value(input).unwrap();
			let action = Policy::apply_regex_response(&mut resp, &rules, &rejection).unwrap();
			(action, serde_json::to_value(&resp.0).unwrap())
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
// A user attaches an HR document and RAG search results; both are message content and
// must be guarded under the default scope.
#[case::anthropic_mask_document_and_search_result(
	ChatFmt::Anthropic,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "summarize the attached"},
				{"type": "document", "title": "roster.txt", "context": "uploaded for ssn 123-45-6789",
					"source": {"type": "text", "media_type": "text/plain", "data": "employee ssn 123-45-6789"}},
				{"type": "search_result", "source": "kb://hr", "title": "HR record", "content": [
					{"type": "text", "text": "ssn 123-45-6789 on file"}
				]}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "summarize the attached"},
				{"type": "document", "title": "roster.txt", "context": "uploaded for ssn <SSN>",
					"source": {"type": "text", "media_type": "text/plain", "data": "employee ssn <SSN>"}},
				{"type": "search_result", "source": "kb://hr", "title": "HR record", "content": [
					{"type": "text", "text": "ssn <SSN> on file"}
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
// Gemini tool traffic: function args, a function response, and a code-execution round trip
// all stay untouched under the default scope; system and visible text are still guarded.
#[case::gemini_mask_tool_output_untouched(
	ChatFmt::Gemini,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"systemInstruction": {"parts": [
			{"text": "You are a helpful assistant. Contact admin@example.com for help."}
		]},
		"contents": [
			{"role": "user", "parts": [{"text": "list pods"}]},
			{"role": "model", "parts": [
				{"text": "Calling the tool."},
				{"functionCall": {"name": "k8s_get_resources", "args": {"ns": "kagent"}}}
			]},
			{"role": "user", "parts": [
				{"functionResponse": {"name": "k8s_get_resources",
					"response": {"output": "pod-a Running, owner ssn 123-45-6789"}}}
			]},
			{"role": "model", "parts": [
				{"executableCode": {"language": "PYTHON", "code": "lookup('owner ssn 123-45-6789')"}},
				{"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "owner ssn 123-45-6789"}}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"systemInstruction": {"parts": [
			{"text": "You are a helpful assistant. Contact <EMAIL_ADDRESS> for help."}
		]},
		"contents": [
			{"role": "user", "parts": [{"text": "list pods"}]},
			{"role": "model", "parts": [
				{"text": "Calling the tool."},
				{"functionCall": {"name": "k8s_get_resources", "args": {"ns": "kagent"}}}
			]},
			{"role": "user", "parts": [
				{"functionResponse": {"name": "k8s_get_resources",
					"response": {"output": "pod-a Running, owner ssn 123-45-6789"}}}
			]},
			{"role": "model", "parts": [
				{"executableCode": {"language": "PYTHON", "code": "lookup('owner ssn 123-45-6789')"}},
				{"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "owner ssn 123-45-6789"}}
			]}
		]
	}))
)]
fn test_apply_regex_preserves_tool_structure(
	#[case] fmt: ChatFmt,
	#[case] action: Action,
	#[case] rules: Vec<RegexRule>,
	#[case] input: serde_json::Value,
	#[case] expected: Expect,
) {
	let (action, actual) = run_apply_regex(fmt, action, rules, input);
	match expected {
		Expect::Masked(expected) => {
			assert_eq!(action, GuardrailAction::Mask);
			assert_eq!(actual, expected);
		},
		Expect::Rejected => assert_eq!(action, GuardrailAction::Reject),
		Expect::Unchanged => assert_eq!(action, GuardrailAction::Allow),
	}
}

#[cfg(test)]
#[rstest::rstest]
#[case::anthropic_mask_tool_output(
	ChatFmt::Anthropic,
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_01", "content": "owner ssn 123-45-6789"}
			]}
		]
	}),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_01", "content": "owner ssn <SSN>"}
			]}
		]
	})
)]
// Replayed server-tool results: bash stdout, a fetched web page, an edit diff, and a
// search result nested in a tool_result — all guardable tool output.
#[case::anthropic_mask_server_tool_results(
	ChatFmt::Anthropic,
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "assistant", "content": [
				{"type": "bash_code_execution_tool_result", "tool_use_id": "srvtoolu_01", "content": {
					"type": "bash_code_execution_result", "stdout": "owner ssn 123-45-6789", "stderr": "", "return_code": 0}},
				{"type": "web_fetch_tool_result", "tool_use_id": "srvtoolu_02", "content": {
					"type": "web_fetch_result", "url": "https://example.com/hr", "content": {
						"type": "document", "title": "HR roster",
						"source": {"type": "text", "media_type": "text/plain", "data": "HR page: ssn 123-45-6789"}}}},
				{"type": "text_editor_code_execution_tool_result", "tool_use_id": "srvtoolu_03", "content": {
					"type": "text_editor_code_execution_str_replace_result",
					"lines": ["- ssn 123-45-6789"], "old_start": 1, "old_lines": 1, "new_start": 1, "new_lines": 1}}
			]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_04", "content": [
					{"type": "search_result", "source": "kb://hr-42", "title": "HR record", "content": [
						{"type": "text", "text": "owner ssn 123-45-6789"}
					]}
				]}
			]}
		]
	}),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "assistant", "content": [
				{"type": "bash_code_execution_tool_result", "tool_use_id": "srvtoolu_01", "content": {
					"type": "bash_code_execution_result", "stdout": "owner ssn <SSN>", "stderr": "", "return_code": 0}},
				{"type": "web_fetch_tool_result", "tool_use_id": "srvtoolu_02", "content": {
					"type": "web_fetch_result", "url": "https://example.com/hr", "content": {
						"type": "document", "title": "HR roster",
						"source": {"type": "text", "media_type": "text/plain", "data": "HR page: ssn <SSN>"}}}},
				{"type": "text_editor_code_execution_tool_result", "tool_use_id": "srvtoolu_03", "content": {
					"type": "text_editor_code_execution_str_replace_result",
					"lines": ["- ssn <SSN>"], "old_start": 1, "old_lines": 1, "new_start": 1, "new_lines": 1}}
			]},
			{"role": "user", "content": [
				{"type": "tool_result", "tool_use_id": "toolu_04", "content": [
					{"type": "search_result", "source": "kb://hr-42", "title": "HR record", "content": [
						{"type": "text", "text": "owner ssn <SSN>"}
					]}
				]}
			]}
		]
	})
)]
#[case::completions_mask_tool_output(
	ChatFmt::Completions,
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "tool", "tool_call_id": "call_01", "content": "owner ssn 123-45-6789"}
		]
	}),
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "tool", "tool_call_id": "call_01", "content": "owner ssn <SSN>"}
		]
	})
)]
#[case::responses_mask_tool_output(
	ChatFmt::Responses,
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "function_call_output", "call_id": "call_01", "output": "owner ssn 123-45-6789"}
		]
	}),
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "function_call_output", "call_id": "call_01", "output": "owner ssn <SSN>"}
		]
	})
)]
// Replayed server-tool items: an MCP server's response and retrieved file-search content
// are both guardable tool output.
#[case::responses_mask_server_tool_results(
	ChatFmt::Responses,
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "mcp_call", "id": "mcp_01", "server_label": "hr", "name": "lookup",
				"arguments": "{\"employee\":\"a\"}", "output": "owner ssn 123-45-6789"},
			{"type": "file_search_call", "id": "fs_01", "queries": ["owner record"], "results": [
				{"file_id": "file_01", "filename": "hr.txt", "text": "ssn 123-45-6789 on file", "score": 0.9}
			]}
		]
	}),
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "mcp_call", "id": "mcp_01", "server_label": "hr", "name": "lookup",
				"arguments": "{\"employee\":\"a\"}", "output": "owner ssn <SSN>"},
			{"type": "file_search_call", "id": "fs_01", "queries": ["owner record"], "results": [
				{"file_id": "file_01", "filename": "hr.txt", "text": "ssn <SSN> on file", "score": 0.9}
			]}
		]
	})
)]
// A function response and replayed code-execution output are both guardable tool output.
#[case::gemini_mask_tool_output(
	ChatFmt::Gemini,
	serde_json::json!({
		"contents": [
			{"role": "user", "parts": [
				{"functionResponse": {"name": "lookup",
					"response": {"output": "owner ssn 123-45-6789"}}},
				{"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "ssn 123-45-6789 in row 1"}}
			]}
		]
	}),
	serde_json::json!({
		"contents": [
			{"role": "user", "parts": [
				{"functionResponse": {"name": "lookup",
					"response": {"output": "owner ssn <SSN>"}}},
				{"codeExecutionResult": {"outcome": "OUTCOME_OK", "output": "ssn <SSN> in row 1"}}
			]}
		]
	})
)]
fn test_apply_regex_tool_output_scope_enabled(
	#[case] fmt: ChatFmt,
	#[case] input: serde_json::Value,
	#[case] expected: serde_json::Value,
) {
	let (action, actual) = run_apply_regex_scoped(fmt, Action::Mask, ssn_only(), all_scopes(), input);
	assert_eq!(action, GuardrailAction::Mask);
	assert_eq!(actual, expected);
}

#[cfg(test)]
fn anthropic_tool_input_request() -> serde_json::Value {
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "assistant", "content": [
				{"type": "text", "text": "Saving the note."},
				{"type": "tool_use", "id": "toolu_01", "name": "save_note", "input": {
					"note": "owner ssn 123-45-6789",
					"tags": ["ssn 123-45-6789"],
					"priority": 1
				}}
			]}
		]
	})
}

#[cfg(test)]
fn completions_tool_input_request() -> serde_json::Value {
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "assistant", "content": null, "tool_calls": [
				{"id": "call_01", "type": "function", "function": {
					"name": "save_note", "arguments": "{\"note\":\"owner ssn 123-45-6789\"}"
				}}
			]}
		]
	})
}

#[cfg(test)]
fn responses_tool_input_request() -> serde_json::Value {
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "function_call", "call_id": "call_01", "name": "save_note",
				"arguments": "{\"note\":\"owner ssn 123-45-6789\"}"}
		]
	})
}

#[cfg(test)]
fn gemini_tool_input_request() -> serde_json::Value {
	serde_json::json!({
		"contents": [
			{"role": "model", "parts": [
				{"text": "Saving the note."},
				{"functionCall": {"name": "save_note", "args": {
					"note": "owner ssn 123-45-6789",
					"tags": ["ssn 123-45-6789"],
					"priority": 1
				}}},
				{"executableCode": {"language": "PYTHON", "code": "save('owner ssn 123-45-6789')"}}
			]}
		]
	})
}

#[cfg(test)]
#[rstest::rstest]
#[case::anthropic_masked_when_enabled(
	ChatFmt::Anthropic,
	all_scopes(),
	anthropic_tool_input_request(),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "assistant", "content": [
				{"type": "text", "text": "Saving the note."},
				{"type": "tool_use", "id": "toolu_01", "name": "save_note", "input": {
					"note": "owner ssn <SSN>",
					"tags": ["ssn <SSN>"],
					"priority": 1
				}}
			]}
		]
	}))
)]
#[case::anthropic_untouched_by_default(
	ChatFmt::Anthropic,
	default_content_scope(),
	anthropic_tool_input_request(),
	Expect::Unchanged
)]
#[case::completions_masked_when_enabled(
	ChatFmt::Completions,
	all_scopes(),
	completions_tool_input_request(),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "assistant", "tool_calls": [
				{"id": "call_01", "type": "function", "function": {
					"name": "save_note", "arguments": "{\"note\":\"owner ssn <SSN>\"}"
				}}
			]}
		]
	}))
)]
#[case::completions_untouched_by_default(
	ChatFmt::Completions,
	default_content_scope(),
	completions_tool_input_request(),
	Expect::Unchanged
)]
#[case::responses_masked_when_enabled(
	ChatFmt::Responses,
	all_scopes(),
	responses_tool_input_request(),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "function_call", "call_id": "call_01", "name": "save_note",
				"arguments": "{\"note\":\"owner ssn <SSN>\"}"}
		]
	}))
)]
#[case::responses_untouched_by_default(
	ChatFmt::Responses,
	default_content_scope(),
	responses_tool_input_request(),
	Expect::Unchanged
)]
// Model-generated code is tool input like function args: guarded only when opted in.
#[case::gemini_masked_when_enabled(
	ChatFmt::Gemini,
	all_scopes(),
	gemini_tool_input_request(),
	Expect::Masked(serde_json::json!({
		"contents": [
			{"role": "model", "parts": [
				{"text": "Saving the note."},
				{"functionCall": {"name": "save_note", "args": {
					"note": "owner ssn <SSN>",
					"tags": ["ssn <SSN>"],
					"priority": 1
				}}},
				{"executableCode": {"language": "PYTHON", "code": "save('owner ssn <SSN>')"}}
			]}
		]
	}))
)]
#[case::gemini_untouched_by_default(
	ChatFmt::Gemini,
	default_content_scope(),
	gemini_tool_input_request(),
	Expect::Unchanged
)]
fn test_apply_regex_tool_input_scope(
	#[case] fmt: ChatFmt,
	#[case] scope: Vec<ContentScope>,
	#[case] input: serde_json::Value,
	#[case] expected: Expect,
) {
	let (action, actual) = run_apply_regex_scoped(fmt, Action::Mask, ssn_only(), scope, input);
	match expected {
		Expect::Masked(expected) => {
			assert_eq!(action, GuardrailAction::Mask);
			assert_eq!(actual, expected);
		},
		Expect::Rejected => assert_eq!(action, GuardrailAction::Reject),
		Expect::Unchanged => assert_eq!(action, GuardrailAction::Allow),
	}
}

#[cfg(test)]
fn anthropic_mcp_request() -> serde_json::Value {
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "assistant", "content": [
				{"type": "mcp_tool_use", "id": "mcptoolu_01", "name": "query_db",
					"server_name": "db", "input": {"query": "ssn 123-45-6789"}}
			]},
			{"role": "user", "content": [
				{"type": "mcp_tool_result", "tool_use_id": "mcptoolu_01", "is_error": false,
					"content": [{"type": "text", "text": "row ssn 123-45-6789"}]}
			]}
		]
	})
}

#[cfg(test)]
fn completions_legacy_function_request() -> serde_json::Value {
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "assistant", "content": null, "function_call": {
				"name": "save_note", "arguments": "{\"note\":\"owner ssn 123-45-6789\"}"
			}},
			{"role": "function", "name": "save_note", "content": "saved ssn 123-45-6789"}
		]
	})
}

#[cfg(test)]
fn responses_mcp_request() -> serde_json::Value {
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "mcp_call", "id": "mcp_01", "server_label": "db", "name": "query",
				"arguments": "{\"q\":\"ssn 123-45-6789\"}", "output": "row ssn 123-45-6789"}
		]
	})
}

#[cfg(test)]
#[rstest::rstest]
#[case::anthropic_mcp_masked_when_enabled(
	ChatFmt::Anthropic,
	all_scopes(),
	anthropic_mcp_request(),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "assistant", "content": [
				{"type": "mcp_tool_use", "id": "mcptoolu_01", "name": "query_db",
					"server_name": "db", "input": {"query": "ssn <SSN>"}}
			]},
			{"role": "user", "content": [
				{"type": "mcp_tool_result", "tool_use_id": "mcptoolu_01", "is_error": false,
					"content": [{"type": "text", "text": "row ssn <SSN>"}]}
			]}
		]
	}))
)]
#[case::anthropic_mcp_untouched_by_default(
	ChatFmt::Anthropic,
	default_content_scope(),
	anthropic_mcp_request(),
	Expect::Unchanged
)]
#[case::anthropic_code_execution_result_masked(
	ChatFmt::Anthropic,
	all_scopes(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "code_execution_tool_result", "tool_use_id": "srvtoolu_01", "content": {
					"type": "code_execution_result", "stdout": "ssn 123-45-6789", "stderr": "", "return_code": 0
				}}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "code_execution_tool_result", "tool_use_id": "srvtoolu_01", "content": {
					"type": "code_execution_result", "stdout": "ssn <SSN>", "stderr": "", "return_code": 0
				}}
			]}
		]
	}))
)]
#[case::completions_legacy_function_masked_when_enabled(
	ChatFmt::Completions,
	all_scopes(),
	completions_legacy_function_request(),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "assistant", "function_call": {
				"name": "save_note", "arguments": "{\"note\":\"owner ssn <SSN>\"}"
			}},
			{"role": "function", "name": "save_note", "content": "saved ssn <SSN>"}
		]
	}))
)]
#[case::completions_legacy_function_untouched_by_default(
	ChatFmt::Completions,
	default_content_scope(),
	completions_legacy_function_request(),
	Expect::Unchanged
)]
#[case::responses_mcp_masked_when_enabled(
	ChatFmt::Responses,
	all_scopes(),
	responses_mcp_request(),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "mcp_call", "id": "mcp_01", "server_label": "db", "name": "query",
				"arguments": "{\"q\":\"ssn <SSN>\"}", "output": "row ssn <SSN>"}
		]
	}))
)]
#[case::responses_mcp_output_scope_only_masks_output(
	ChatFmt::Responses,
	vec![ContentScope::ToolOutput],
	responses_mcp_request(),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"type": "mcp_call", "id": "mcp_01", "server_label": "db", "name": "query",
				"arguments": "{\"q\":\"ssn 123-45-6789\"}", "output": "row ssn <SSN>"}
		]
	}))
)]
#[case::responses_mcp_untouched_by_default(
	ChatFmt::Responses,
	default_content_scope(),
	responses_mcp_request(),
	Expect::Unchanged
)]
fn test_apply_regex_provider_tool_items(
	#[case] fmt: ChatFmt,
	#[case] scope: Vec<ContentScope>,
	#[case] input: serde_json::Value,
	#[case] expected: Expect,
) {
	let (action, actual) = run_apply_regex_scoped(fmt, Action::Mask, ssn_only(), scope, input);
	match expected {
		Expect::Masked(expected) => {
			assert_eq!(action, GuardrailAction::Mask);
			assert_eq!(actual, expected);
		},
		Expect::Rejected => assert_eq!(action, GuardrailAction::Reject),
		Expect::Unchanged => assert_eq!(action, GuardrailAction::Allow),
	}
}

#[test]
fn request_guard_scope_rejects_empty_list() {
	let err = serde_json::from_value::<RequestGuard>(serde_json::json!({
		"regex": {"action": "mask", "rules": [{"builtin": "ssn"}]},
		"scope": [],
	}))
	.unwrap_err();
	assert!(err.to_string().contains("scope must not be empty"), "{err}");

	let guard = serde_json::from_value::<RequestGuard>(serde_json::json!({
		"regex": {"action": "mask", "rules": [{"builtin": "ssn"}]},
	}))
	.unwrap();
	assert_eq!(guard.scope, default_content_scope());
}

#[test]
fn prompt_guard_scope_support() {
	// claiming tool coverage on a guard that only ever sees message text must not parse
	let err = serde_json::from_value::<PromptGuard>(serde_json::json!({
		"request": [{
			"openAIModeration": {},
			"scope": ["messages", "toolInput"],
		}]
	}))
	.unwrap_err();
	assert!(err.to_string().contains("non-default scope"), "{err}");

	// spelling out the default is honest, in any order
	serde_json::from_value::<PromptGuard>(serde_json::json!({
		"request": [{
			"openAIModeration": {},
			"scope": ["messages", "systemPrompt"],
		}]
	}))
	.unwrap();

	serde_json::from_value::<PromptGuard>(serde_json::json!({
		"request": [{
			"bedrockGuardrails": {
				"guardrailIdentifier": "gr-1",
				"guardrailVersion": "1",
				"region": "us-east-1",
			},
			"scope": ["messages", "toolOutput"],
		}]
	}))
	.unwrap();
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
	let (action, actual) = run_apply_regex_response(fmt, action, rules, input);
	match expected {
		Some(expected) => {
			assert_eq!(action, GuardrailAction::Mask);
			assert_eq!(actual, expected);
		},
		None => assert_eq!(action, GuardrailAction::Reject),
	}
}

#[cfg(test)]
#[rstest::rstest]
#[case::completions_reject_across_blocks(
	ChatFmt::Completions,
	Action::Reject,
	vec![RegexRule::Regex { pattern: regex::Regex::new("secret token").unwrap() }],
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "Please use this secret"},
				{"type": "text", "text": "token to authenticate"}
			]}
		]
	}),
	Expect::Rejected
)]
#[case::anthropic_reject_across_blocks(
	ChatFmt::Anthropic,
	Action::Reject,
	vec![RegexRule::Regex { pattern: regex::Regex::new("secret token").unwrap() }],
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "Please use this secret"},
				{"type": "text", "text": "token to authenticate"}
			]}
		]
	}),
	Expect::Rejected
)]
#[case::responses_reject_across_blocks(
	ChatFmt::Responses,
	Action::Reject,
	vec![RegexRule::Regex { pattern: regex::Regex::new("secret\ntoken").unwrap() }],
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"role": "user", "content": [
				{"type": "input_text", "text": "Please use this secret"},
				{"type": "input_text", "text": "token to authenticate"}
			]}
		]
	}),
	Expect::Rejected
)]
#[case::completions_mask_collapses_run(
	ChatFmt::Completions,
	Action::Mask,
	vec![RegexRule::Regex { pattern: regex::Regex::new("secret token").unwrap() }],
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "Please use this secret"},
				{"type": "text", "text": "token to authenticate"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "Please use this <masked> to authenticate"}
			]}
		]
	}))
)]
#[case::anthropic_mask_collapsed_run_keeps_last_blocks_cache_control(
	ChatFmt::Anthropic,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact admin@example.com"},
				{"type": "text", "text": "for help", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact <EMAIL_ADDRESS> for help", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}))
)]
// the common caching layout: breakpoint on a big context block, volatile text after it
#[case::mask_carries_drained_blocks_cache_control(
	ChatFmt::Anthropic,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact admin@example.com", "cache_control": {"type": "ephemeral"}},
				{"type": "text", "text": "for help"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact <EMAIL_ADDRESS> for help", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}))
)]
// two breakpoints collapse into one; the later one wins
#[case::mask_keeps_survivors_cache_control_over_drained(
	ChatFmt::Anthropic,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact admin@example.com", "cache_control": {"type": "ephemeral", "ttl": "1h"}},
				{"type": "text", "text": "for help", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact <EMAIL_ADDRESS> for help", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}))
)]
#[case::system_array_mask_carries_cache_control(
	ChatFmt::Anthropic,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"system": [
			{"type": "text", "text": "Contact admin@example.com", "cache_control": {"type": "ephemeral"}},
			{"type": "text", "text": "for support"}
		],
		"messages": [{"role": "user", "content": "hello"}]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"system": [
			{"type": "text", "text": "Contact <EMAIL_ADDRESS>\nfor support", "cache_control": {"type": "ephemeral"}}
		],
		"messages": [{"role": "user", "content": "hello"}]
	}))
)]
// OpenAI-format providers (Bedrock, OpenRouter) accept cache_control on content parts
#[case::completions_mask_carries_cache_control(
	ChatFmt::Completions,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "my ssn is 123-45-6789", "cache_control": {"type": "ephemeral"}},
				{"type": "text", "text": "thanks"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "my ssn is <SSN> thanks", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}))
)]
// OpenAI explicit breakpoints (prompt_cache_breakpoint) survive a collapse too
#[case::responses_mask_carries_prompt_cache_breakpoint(
	ChatFmt::Responses,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"role": "user", "content": [
				{"type": "input_text", "text": "my ssn is 123-45-6789", "prompt_cache_breakpoint": {"mode": "explicit"}},
				{"type": "input_text", "text": "thanks"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"input": [
			{"role": "user", "content": [
				{"type": "input_text", "text": "my ssn is <SSN>\nthanks", "prompt_cache_breakpoint": {"mode": "explicit"}}
			]}
		]
	}))
)]
#[case::completions_mask_carries_prompt_cache_breakpoint(
	ChatFmt::Completions,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "my ssn is 123-45-6789", "prompt_cache_breakpoint": {"mode": "explicit"}},
				{"type": "text", "text": "thanks"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "my ssn is <SSN> thanks", "prompt_cache_breakpoint": {"mode": "explicit"}}
			]}
		]
	}))
)]
// a masked run must not steal the breakpoint from an untouched run before the image
#[case::mask_does_not_leak_cache_control_across_runs(
	ChatFmt::Anthropic,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "project context", "cache_control": {"type": "ephemeral"}},
				{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGk="}},
				{"type": "text", "text": "my ssn is 123-45-6789"},
				{"type": "text", "text": "thanks"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "project context", "cache_control": {"type": "ephemeral"}},
				{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGk="}},
				{"type": "text", "text": "my ssn is <SSN> thanks"}
			]}
		]
	}))
)]
// two drained parts both marked: the later breakpoint is the one carried
#[case::mask_carries_latest_drained_cache_control(
	ChatFmt::Anthropic,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact admin@example.com", "cache_control": {"type": "ephemeral", "ttl": "1h"}},
				{"type": "text", "text": "for help", "cache_control": {"type": "ephemeral"}},
				{"type": "text", "text": "thanks"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact <EMAIL_ADDRESS> for help thanks", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}))
)]
// an explicit null marker is not a breakpoint and must not shadow the real one
#[case::null_cache_control_does_not_shadow_real_marker(
	ChatFmt::Anthropic,
	Action::Mask,
	email_and_ssn(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact admin@example.com", "cache_control": {"type": "ephemeral"}},
				{"type": "text", "text": "for help", "cache_control": null}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "contact <EMAIL_ADDRESS> for help", "cache_control": {"type": "ephemeral"}}
			]}
		]
	}))
)]
#[case::mask_preserves_non_text_block_between_runs(
	ChatFmt::Anthropic,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "my ssn is 123-45-6789"},
				{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGk="}},
				{"type": "text", "text": "thanks"}
			]}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "claude-sonnet-5",
		"max_tokens": 1024,
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "my ssn is <SSN>"},
				{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "aGk="}},
				{"type": "text", "text": "thanks"}
			]}
		]
	}))
)]
#[case::untouched_run_is_not_collapsed(
	ChatFmt::Completions,
	Action::Mask,
	ssn_only(),
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "no pii"},
				{"type": "text", "text": "in here"}
			]},
			{"role": "user", "content": "my ssn is 123-45-6789"}
		]
	}),
	Expect::Masked(serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": [
				{"type": "text", "text": "no pii"},
				{"type": "text", "text": "in here"}
			]},
			{"role": "user", "content": "my ssn is <SSN>"}
		]
	}))
)]
#[case::no_match_across_separate_messages(
	ChatFmt::Completions,
	Action::Reject,
	vec![RegexRule::Regex { pattern: regex::Regex::new("secret token").unwrap() }],
	serde_json::json!({
		"model": "gpt-4o",
		"messages": [
			{"role": "user", "content": "Please use this secret"},
			{"role": "user", "content": "token to authenticate"}
		]
	}),
	Expect::Unchanged
)]
fn test_apply_regex_text_runs(
	#[case] fmt: ChatFmt,
	#[case] action: Action,
	#[case] rules: Vec<RegexRule>,
	#[case] input: serde_json::Value,
	#[case] expected: Expect,
) {
	let (action, actual) = run_apply_regex(fmt, action, rules, input);
	match expected {
		Expect::Masked(expected) => {
			assert_eq!(action, GuardrailAction::Mask);
			assert_eq!(actual, expected);
		},
		Expect::Rejected => assert_eq!(action, GuardrailAction::Reject),
		Expect::Unchanged => assert_eq!(action, GuardrailAction::Allow),
	}
}

#[cfg(test)]
#[test]
fn test_zero_width_pattern_is_a_noop() {
	let (action, actual) = run_apply_regex(
		ChatFmt::Completions,
		Action::Mask,
		vec![RegexRule::Regex {
			pattern: regex::Regex::new("z*").unwrap(),
		}],
		serde_json::json!({
			"model": "gpt-4o",
			"messages": [{"role": "user", "content": "hello world"}]
		}),
	);
	assert_eq!(action, GuardrailAction::Allow);
	assert_eq!(
		actual,
		serde_json::json!({
			"model": "gpt-4o",
			"messages": [{"role": "user", "content": "hello world"}]
		})
	);
}
