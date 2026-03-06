use std::fs;
use std::path::Path;
use std::sync::Arc;

use secrecy::ExposeSecret;

use super::*;
use crate::types::agent::HeaderValueMatch;
use crate::types::local::NormalizedLocalConfig;

fn make_test_client() -> crate::client::Client {
	let cfg = crate::client::Config {
		resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
		resolver_opts: hickory_resolver::config::ResolverOpts::default(),
	};
	crate::client::Client::new(&cfg, None, Default::default(), None)
}

async fn test_config_parsing(test_name: &str) {
	// Make it static
	super::STARTUP_TIMESTAMP.get_or_init(|| 0);
	let test_dir = Path::new("src/types/local_tests");
	let input_path = test_dir.join(format!("{}_config.yaml", test_name));

	let yaml_str = fs::read_to_string(&input_path).unwrap();

	let client = make_test_client();
	let config = crate::config::parse_config("{}".to_string(), None).unwrap();

	let normalized = NormalizedLocalConfig::from(
		&config,
		client,
		Arc::new(crate::http::oidc::OidcClient::new()),
		ListenerTarget {
			gateway_name: "name".into(),
			gateway_namespace: "ns".into(),
			listener_name: None,
		},
		&yaml_str,
	)
	.await
	.unwrap_or_else(|e| panic!("Failed to normalize config from: {:?} {e}", input_path));

	insta::with_settings!({
		description => format!("Config normalization test for {}: YAML -> LocalConfig -> NormalizedLocalConfig -> YAML", test_name),
		omit_expression => true,
		prepend_module_to_snapshot => false,
		snapshot_path => "local_tests",
		sort_maps => true,
	}, {
		insta::assert_yaml_snapshot!(format!("{}_normalized", test_name), normalized);
	});
}

#[tokio::test]
async fn test_basic_config() {
	test_config_parsing("basic").await;
}

#[tokio::test]
async fn test_mcp_config() {
	test_config_parsing("mcp").await;
}

#[tokio::test]
async fn test_llm_config() {
	test_config_parsing("llm").await;
}

#[tokio::test]
async fn test_llm_simple_config() {
	test_config_parsing("llm_simple").await;
}

#[tokio::test]
async fn test_mcp_simple_config() {
	test_config_parsing("mcp_simple").await;
}

#[tokio::test]
async fn test_aws_config() {
	test_config_parsing("aws").await;
}

#[tokio::test]
async fn test_health_config() {
	test_config_parsing("health").await;
}

#[test]
fn test_llm_model_name_header_match_valid_patterns() {
	match super::llm_model_name_header_match("*").unwrap() {
		HeaderValueMatch::Regex(re) => assert_eq!(re.as_str(), ".*"),
		other => panic!("expected regex for '*', got {other:?}"),
	}

	match super::llm_model_name_header_match("*gpt-4.1").unwrap() {
		HeaderValueMatch::Regex(re) => assert_eq!(re.as_str(), ".*gpt\\-4\\.1"),
		other => panic!("expected regex for '*gpt-4.1', got {other:?}"),
	}

	match super::llm_model_name_header_match("gpt-4.1*").unwrap() {
		HeaderValueMatch::Regex(re) => assert_eq!(re.as_str(), "gpt\\-4\\.1.*"),
		other => panic!("expected regex for 'gpt-4.1*', got {other:?}"),
	}

	match super::llm_model_name_header_match("gpt-4.1").unwrap() {
		HeaderValueMatch::Exact(v) => assert_eq!(v, ::http::HeaderValue::from_static("gpt-4.1")),
		other => panic!("expected exact header value for 'gpt-4.1', got {other:?}"),
	}
}

#[test]
fn test_llm_model_name_header_match_invalid_patterns() {
	assert!(super::llm_model_name_header_match("*gpt*").is_err());
	assert!(super::llm_model_name_header_match("g*pt").is_err());
}

#[test]
fn test_migrate_deprecated_local_config_moves_fields() {
	let input = r#"
config:
  logging:
    level: info
    filter: request.path == "/foo"
    fields:
      remove:
        - foo
      add:
        region: request.host
  tracing:
    otlpEndpoint: otlp.default.svc.cluster.local:4317
    headers:
      authorization: token
    otlpProtocol: http
"#;
	let out = super::migrate_deprecated_local_config(input).unwrap();
	let v: serde_json::Value = crate::serdes::yamlviajson::from_str(&out).unwrap();
	let cfg = v.get("config").unwrap();
	let logging = cfg.get("logging").unwrap();
	assert_eq!(logging.get("level").unwrap(), "info");
	assert!(logging.get("filter").is_none());
	assert!(logging.get("fields").is_none());
	assert!(cfg.get("tracing").is_none());
	let frontend = v.get("frontendPolicies").unwrap();
	assert!(frontend.get("logging").is_none());
	let access_log = frontend.get("accessLog").unwrap();
	assert_eq!(
		access_log.get("filter").unwrap(),
		"request.path == \"/foo\""
	);
	assert_eq!(
		access_log.get("add").unwrap().get("region").unwrap(),
		"request.host"
	);
	assert_eq!(access_log.get("remove").unwrap()[0], "foo");
	let tracing = frontend.get("tracing").unwrap();
	assert_eq!(
		tracing.get("inlineBackend").unwrap(),
		"otlp.default.svc.cluster.local:4317"
	);
	assert_eq!(tracing.get("protocol").unwrap(), "http");
}

#[test]
fn local_oauth2_policy_reads_supported_client_secret_sources() {
	let dir = tempfile::tempdir().expect("temp dir");
	let secret_path = dir.path().join("oauth2-client-secret");
	std::fs::write(&secret_path, "from-file-secret").expect("write secret");

	let cases = [
		(
			"inline",
			serde_json::json!({
				"issuer": "https://issuer.example.com",
				"clientId": "client-id",
				"clientSecret": "super-secret",
				"redirectUri": "https://issuer.example.com/_gateway/callback",
			}),
			"super-secret",
		),
		(
			"file",
			serde_json::json!({
				"issuer": "https://issuer.example.com",
				"clientId": "client-id",
				"clientSecret": { "file": secret_path },
				"redirectUri": "https://issuer.example.com/_gateway/callback",
			}),
			"from-file-secret",
		),
	];

	for (name, value, expected_secret) in cases {
		let policy: LocalOAuth2Policy = serde_json::from_value(value).expect("policy should parse");
		let oauth2 = policy.try_into().expect("policy should convert");
		assert_eq!(
			oauth2.client_secret.expose_secret(),
			expected_secret,
			"case {name:?} produced unexpected client secret",
		);
	}
}

#[test]
fn local_oauth2_policy_accepts_resolved_provider_without_jwks() {
	let policy: LocalOAuth2Policy = serde_json::from_value(serde_json::json!({
		"authorizationEndpoint": "https://issuer.example.com/authorize",
		"tokenEndpoint": "https://issuer.example.com/token",
		"clientId": "client-id",
		"clientSecret": "super-secret",
		"redirectUri": "https://issuer.example.com/_gateway/callback",
	}))
	.expect("policy should parse");

	let oauth2 = policy.try_into().expect("policy should convert");
	let resolved = oauth2
		.resolved_provider
		.expect("resolved provider should be present");
	assert_eq!(
		resolved.authorization_endpoint,
		"https://issuer.example.com/authorize"
	);
	assert_eq!(resolved.token_endpoint, "https://issuer.example.com/token");
	assert_eq!(resolved.jwks_inline, None);
}

#[test]
fn local_oauth2_policy_rejects_schema_and_conversion_errors() {
	struct Case {
		name: &'static str,
		value: serde_json::Value,
		want_err: &'static str,
		parse_should_fail: bool,
	}

	let cases = [
		Case {
			name: "missing client secret",
			value: serde_json::json!({
				"issuer": "https://issuer.example.com",
				"clientId": "client-id",
			}),
			want_err: "clientSecret",
			parse_should_fail: true,
		},
		Case {
			name: "client secret ref only",
			value: serde_json::json!({
				"issuer": "https://issuer.example.com",
				"clientId": "client-id",
				"clientSecretRef": {
					"name": "oauth2-client-secret",
					"key": "client-secret",
				},
			}),
			want_err: "clientSecret",
			parse_should_fail: true,
		},
		Case {
			name: "missing redirect uri",
			value: serde_json::json!({
				"issuer": "https://issuer.example.com",
				"clientId": "client-id",
				"clientSecret": "super-secret",
			}),
			want_err: "missing field `redirectUri`",
			parse_should_fail: true,
		},
		Case {
			name: "missing provider mode",
			value: serde_json::json!({
				"cookieName": "test",
				"clientId": "client-id",
				"clientSecret": "super-secret",
				"redirectUri": "https://issuer.example.com/_gateway/callback",
			}),
			want_err: "issuer or both authorizationEndpoint and tokenEndpoint",
			parse_should_fail: false,
		},
	];

	for case in cases {
		if case.parse_should_fail {
			let err = serde_json::from_value::<LocalOAuth2Policy>(case.value)
				.expect_err("policy parse should fail");
			assert!(
				err.to_string().contains(case.want_err),
				"case {:?}: unexpected error: {err}",
				case.name,
			);
			continue;
		}

		let policy: LocalOAuth2Policy =
			serde_json::from_value(case.value).expect("policy should parse");
		let err = policy
			.try_into()
			.expect_err("policy conversion should fail");
		assert!(
			err.to_string().contains(case.want_err),
			"case {:?}: unexpected error: {err}",
			case.name,
		);
	}
}

#[tokio::test]
async fn split_policies_translates_local_oauth2_client_secret() {
	let policy: LocalOAuth2Policy = crate::serdes::yamlviajson::from_str(
		r#"
authorizationEndpoint: https://issuer.example.com/authorize
tokenEndpoint: https://issuer.example.com/token
clientId: client-id
clientSecret: secret-from-inline
redirectUri: https://issuer.example.com/_gateway/callback
"#,
	)
	.expect("policy should parse");
	let filter_or_policy = FilterOrPolicy {
		oauth2: Some(policy),
		..Default::default()
	};

	let resolved = split_policies(
		make_test_client(),
		Arc::new(crate::http::oidc::OidcClient::new()),
		filter_or_policy,
	)
	.await
	.expect("split_policies should succeed");

	let [TrafficPolicy::OAuth2(oauth2)] = resolved.route_policies.as_slice() else {
		panic!("expected exactly one oauth2 route policy");
	};
	assert_eq!(
		oauth2.config().client_secret.expose_secret(),
		"secret-from-inline"
	);
}

#[test]
fn local_oauth2_policy_maps_hardening_fields() {
	let policy: LocalOAuth2Policy = serde_json::from_value(serde_json::json!({
		"issuer": "https://issuer.example.com",
		"clientId": "client-id",
		"clientSecret": "super-secret",
		"redirectUri": "https://issuer.example.com/_gateway/callback",
		"refreshableCookieMaxAgeSeconds": 900,
		"postLogoutRedirectUri": "https://app.example.com/signed-out",
	}))
	.expect("policy should parse");

	let oauth2 = policy.try_into().expect("policy should convert");
	assert_eq!(
		oauth2.redirect_uri.as_deref(),
		Some("https://issuer.example.com/_gateway/callback")
	);
	assert_eq!(oauth2.refreshable_cookie_max_age_seconds, Some(900));
	assert_eq!(
		oauth2.post_logout_redirect_uri.as_deref(),
		Some("https://app.example.com/signed-out")
	);
}
