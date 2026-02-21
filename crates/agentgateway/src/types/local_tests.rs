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
	crate::client::Client::new(
		&cfg,
		None,
		Default::default(),
		None,
		Arc::new(crate::http::oidc::OidcProvider::new()),
	)
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

#[tokio::test]
async fn local_oauth2_policy_accepts_inline_client_secret() {
	let policy: LocalOAuth2Policy = crate::serdes::yamlviajson::from_str(
		r#"
issuer: https://issuer.example.com
clientId: client-id
clientSecret: super-secret
autoDetectRedirectUri: true
"#,
	)
	.expect("policy should parse");

	let oauth2 = policy.try_into().expect("policy should convert");
	assert_eq!(oauth2.client_secret.expose_secret(), "super-secret");
}

#[tokio::test]
async fn local_oauth2_policy_loads_client_secret_from_file() {
	let dir = tempfile::tempdir().expect("temp dir");
	let secret_path = dir.path().join("oauth2-client-secret");
	std::fs::write(&secret_path, "from-file-secret").expect("write secret");

	let policy: LocalOAuth2Policy = serde_json::from_value(serde_json::json!({
		"issuer": "https://issuer.example.com",
		"clientId": "client-id",
		"clientSecret": { "file": secret_path },
		"autoDetectRedirectUri": true,
	}))
	.expect("policy should parse");

	let oauth2 = policy.try_into().expect("policy should convert");
	assert_eq!(oauth2.client_secret.expose_secret(), "from-file-secret");
}

#[tokio::test]
async fn local_oauth2_policy_rejects_missing_client_secret() {
	let err = serde_json::from_value::<LocalOAuth2Policy>(serde_json::json!({
		"issuer": "https://issuer.example.com",
		"clientId": "client-id",
	}))
	.expect_err("policy parse should fail");
	assert!(
		err.to_string().contains("clientSecret"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn local_oauth2_policy_rejects_client_secret_ref_only() {
	let err = serde_json::from_value::<LocalOAuth2Policy>(serde_json::json!({
		"issuer": "https://issuer.example.com",
		"clientId": "client-id",
		"clientSecretRef": {
			"name": "oauth2-client-secret",
			"key": "client-secret",
		},
	}))
	.expect_err("policy parse should fail");
	assert!(
		err.to_string().contains("clientSecret"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn local_oauth2_policy_requires_redirect_or_auto_detect() {
	let policy: LocalOAuth2Policy = serde_json::from_value(serde_json::json!({
		"issuer": "https://issuer.example.com",
		"clientId": "client-id",
		"clientSecret": "super-secret",
	}))
	.expect("policy should parse");

	let err = policy
		.try_into()
		.expect_err("missing redirect configuration must fail");
	assert!(
		err
			.to_string()
			.contains("redirect_uri or auto_detect_redirect_uri=true"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn local_oauth2_policy_rejects_excessive_refreshable_cookie_max_age() {
	let policy: LocalOAuth2Policy = serde_json::from_value(serde_json::json!({
		"issuer": "https://issuer.example.com",
		"clientId": "client-id",
		"clientSecret": "super-secret",
		"autoDetectRedirectUri": true,
		"refreshableCookieMaxAgeSeconds": 2_592_001,
	}))
	.expect("policy should parse");

	let err = policy
		.try_into()
		.expect_err("excessive refreshable max age must fail");
	assert!(
		err
			.to_string()
			.contains("refreshable_cookie_max_age_seconds must be <="),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn split_policies_translates_local_oauth2_client_secret() {
	let policy: LocalOAuth2Policy = crate::serdes::yamlviajson::from_str(
		r#"
issuer: https://issuer.example.com
clientId: client-id
clientSecret: secret-from-inline
autoDetectRedirectUri: true
"#,
	)
	.expect("policy should parse");
	let filter_or_policy = FilterOrPolicy {
		oauth2: Some(policy),
		..Default::default()
	};

	let resolved = split_policies(make_test_client(), filter_or_policy)
		.await
		.expect("split_policies should succeed");

	let [TrafficPolicy::OAuth2(oauth2)] = resolved.route_policies.as_slice() else {
		panic!("expected exactly one oauth2 route policy");
	};
	assert_eq!(oauth2.client_secret.expose_secret(), "secret-from-inline");
}

#[tokio::test]
async fn local_oauth2_policy_maps_hardening_fields() {
	let policy: LocalOAuth2Policy = serde_json::from_value(serde_json::json!({
		"issuer": "https://issuer.example.com",
		"clientId": "client-id",
		"clientSecret": "super-secret",
		"autoDetectRedirectUri": true,
		"refreshableCookieMaxAgeSeconds": 900,
		"postLogoutRedirectUri": "https://app.example.com/signed-out",
		"denyRedirectMatchers": ["/api"],
		"trustedProxyCidrs": ["10.0.0.0/8"],
	}))
	.expect("policy should parse");

	let oauth2 = policy.try_into().expect("policy should convert");
	assert_eq!(oauth2.auto_detect_redirect_uri, Some(true));
	assert_eq!(oauth2.refreshable_cookie_max_age_seconds, Some(900));
	assert_eq!(
		oauth2.post_logout_redirect_uri.as_deref(),
		Some("https://app.example.com/signed-out")
	);
	assert_eq!(oauth2.deny_redirect_matchers, vec!["/api"]);
	assert_eq!(oauth2.trusted_proxy_cidrs, vec!["10.0.0.0/8"]);
}
