use crate::serdes::FileInlineOrRemote;
use crate::types::agent::HeaderValueMatch;
use crate::types::agent::{
	ListenerTarget, PolicyPhase, PolicyTarget, PolicyType, ResourceName, TrafficPolicy,
};
use crate::types::local::NormalizedLocalConfig;
use crate::*;
use secrecy::SecretString;
use std::fs;
use std::path::Path;
use std::sync::Arc;

const TEST_OIDC_JWKS: &str = r#"{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}"#;

fn test_client() -> client::Client {
	client::Client::new(
		&client::Config {
			resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
			resolver_opts: hickory_resolver::config::ResolverOpts::default(),
		},
		None,
		BackendConfig::default(),
		None,
	)
}

fn test_config() -> crate::Config {
	let mut config = crate::config::parse_config("{}".to_string(), None).unwrap();
	config.oidc_cookie_encoder = Some(
		crate::http::sessionpersistence::Encoder::aes(
			"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		)
		.expect("aes encoder"),
	);
	config
}

fn test_oidc_policy() -> super::FilterOrPolicy {
	super::FilterOrPolicy {
		oidc: Some(crate::http::oidc::LocalOidcConfig {
			issuer: "https://issuer.example.com".into(),
			discovery: None,
			authorization_endpoint: Some(
				"https://issuer.example.com/authorize"
					.parse()
					.expect("authorization endpoint"),
			),
			token_endpoint: Some(
				"https://issuer.example.com/token"
					.parse()
					.expect("token endpoint"),
			),
			token_endpoint_auth: None,
			jwks: Some(FileInlineOrRemote::Inline(TEST_OIDC_JWKS.to_string())),
			client_id: "client-id".into(),
			client_secret: SecretString::new("client-secret".into()),
			redirect_uri: "http://localhost:3000/oauth/callback".into(),
			scopes: vec![],
		}),
		..Default::default()
	}
}

async fn normalize_test_policies(
	policies: Vec<super::LocalPolicy>,
) -> anyhow::Result<super::NormalizedLocalConfig> {
	super::convert(
		test_client(),
		ListenerTarget {
			gateway_name: "name".into(),
			gateway_namespace: "ns".into(),
			listener_name: None,
			port: None,
		},
		&test_config(),
		super::LocalConfig {
			config: Arc::new(None),
			binds: vec![],
			frontend_policies: Default::default(),
			policies,
			workloads: vec![],
			services: vec![],
			backends: vec![],
			route_groups: vec![],
			llm: None,
			mcp: None,
		},
	)
	.await
}

async fn normalize_test_yaml(yaml: &str) -> anyhow::Result<NormalizedLocalConfig> {
	NormalizedLocalConfig::from(
		&test_config(),
		test_client(),
		ListenerTarget {
			gateway_name: "name".into(),
			gateway_namespace: "ns".into(),
			listener_name: None,
			port: None,
		},
		yaml,
	)
	.await
}

async fn normalize_test_config(yaml_str: &str) -> anyhow::Result<NormalizedLocalConfig> {
	let client = test_client();
	let config = crate::config::parse_config(yaml_str.to_string(), None).unwrap();

	NormalizedLocalConfig::from(
		&config,
		client,
		ListenerTarget {
			gateway_name: "name".into(),
			gateway_namespace: "ns".into(),
			listener_name: None,
			port: None,
		},
		yaml_str,
	)
	.await
}

async fn test_config_parsing(test_name: &str) {
	// Make it static
	super::STARTUP_TIMESTAMP.get_or_init(|| 0);
	let test_dir = Path::new("src/types/local_tests");
	let input_path = test_dir.join(format!("{}_config.yaml", test_name));

	let yaml_str = fs::read_to_string(&input_path).unwrap();
	let normalized = normalize_test_config(&yaml_str)
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
async fn test_named_mcp_backend_config() {
	test_config_parsing("named_mcp_backend").await;
}

#[tokio::test]
async fn test_mcp_to_aws_backend_config() {
	test_config_parsing("mcp_to_aws_backend").await;
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
async fn test_local_mcp_target_name_wiring_rejects_plus() {
	let yaml = r#"
mcp:
  targets:
  - name: "bad+name"
    stdio:
      cmd: echo
"#;
	normalize_test_yaml(yaml)
		.await
		.expect_err("MCP target name containing '+' should be rejected");
}

#[tokio::test]
async fn test_aws_config() {
	test_config_parsing("aws").await;
}

#[tokio::test]
async fn test_health_config() {
	test_config_parsing("health").await;
}

#[tokio::test]
async fn test_inference_routing_requires_service_backend() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - host: 127.0.0.1:8000
        policies:
          inferenceRouting:
            endpointPicker:
              host: 127.0.0.1:9002
"#;

	let err = normalize_test_config(input).await.unwrap_err();
	assert!(
		err
			.to_string()
			.contains("inferenceRouting is only supported on service route backends"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn test_inference_routing_service_backend_config() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - service:
          name: default/my-model
          port: 8000
        policies:
          inferenceRouting:
            endpointPicker:
              host: 127.0.0.1:9002
"#;

	normalize_test_config(input)
		.await
		.expect("service backends should allow inference routing");
}

#[tokio::test]
async fn test_inference_routing_rejects_failure_mode() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - service:
          name: default/my-model
          port: 8000
        policies:
          inferenceRouting:
            endpointPicker:
              host: 127.0.0.1:9002
            failureMode: failOpen
"#;

	let err = normalize_test_config(input).await.unwrap_err();
	assert!(
		err.to_string().contains("failureMode"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn test_inference_routing_rejects_named_backend_policies() {
	let input = r#"
backends:
- name: model
  host: 127.0.0.1:8000
  policies:
    inferenceRouting:
      endpointPicker:
        host: 127.0.0.1:9002
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - backend: model
"#;

	let err = normalize_test_config(input).await.unwrap_err();
	assert!(
		err
			.to_string()
			.contains("inferenceRouting is only supported on service route backends, not named backends"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn test_inference_routing_rejects_ai_provider_policies() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - ai:
          name: openai
          provider:
            openAI: {}
          policies:
            inferenceRouting:
              endpointPicker:
                host: 127.0.0.1:9002
"#;

	let err = normalize_test_config(input).await.unwrap_err();
	assert!(
		err.to_string().contains(
			"inferenceRouting is only supported on service route backends, not AI provider policies"
		),
		"unexpected error: {err}"
	);
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

#[tokio::test]
async fn test_targeted_gateway_phase_oidc_accepts_gateway_and_listener_targets() {
	for target in [
		PolicyTarget::Gateway(ListenerTarget {
			gateway_name: "name".into(),
			gateway_namespace: "ns".into(),
			listener_name: None,
			port: None,
		}),
		PolicyTarget::Gateway(ListenerTarget {
			gateway_name: "name".into(),
			gateway_namespace: "ns".into(),
			listener_name: Some("listener".into()),
			port: None,
		}),
	] {
		let normalized = normalize_test_policies(vec![super::LocalPolicy {
			name: ResourceName::new("oidc".into(), "default".into()),
			target,
			phase: PolicyPhase::Gateway,
			policy: test_oidc_policy(),
		}])
		.await
		.expect("gateway/listener target should accept gateway-phase oidc");

		let policy = normalized.policies.first().expect("normalized policy");
		match &policy.policy {
			PolicyType::Traffic(traffic) => {
				assert_eq!(traffic.phase, PolicyPhase::Gateway);
				assert!(matches!(traffic.policy, TrafficPolicy::Oidc(_)));
			},
			other => panic!("expected traffic policy, got {other:?}"),
		}
	}
}

#[tokio::test]
async fn test_listener_gateway_policy_surface_supports_oidc() {
	let normalized = normalize_test_yaml(&format!(
		r#"
binds:
- port: 3000
  listeners:
  - policies:
      oidc:
        issuer: https://issuer.example.com
        authorizationEndpoint: https://issuer.example.com/authorize
        tokenEndpoint: https://issuer.example.com/token
        jwks: '{TEST_OIDC_JWKS}'
        clientId: client-id
        clientSecret: client-secret
        redirectURI: http://localhost:3000/oauth/callback
    routes:
    - backends:
      - host: 127.0.0.1:8080
"#
	))
	.await
	.expect("listener policies should normalize gateway-phase oidc");

	assert!(normalized.policies.iter().any(|policy| {
		matches!(
			&policy.policy,
			PolicyType::Traffic(traffic)
				if traffic.phase == PolicyPhase::Gateway
					&& matches!(traffic.policy, TrafficPolicy::Oidc(_))
		)
	}));
}

#[tokio::test]
async fn test_mcp_named_backend_reference_reuses_existing_backend() {
	let normalized = normalize_test_yaml(
		r#"
backends:
- name: shared-upstream
  host: example.com:443
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - mcp:
          targets:
          - name: remote
            mcp:
              backend: shared-upstream
              path: /mcp
"#,
	)
	.await
	.expect("named MCP backend reference should normalize");

	assert_eq!(
		normalized.backends.len(),
		2,
		"should keep the explicit backend plus the MCP backend, without a synthetic target backend"
	);

	let mcp_backend = normalized
		.backends
		.iter()
		.find(|backend| matches!(backend.backend, crate::types::agent::Backend::MCP(_, _)))
		.expect("normalized MCP backend");

	let crate::types::agent::Backend::MCP(_, mcp_backend) = &mcp_backend.backend else {
		panic!("expected MCP backend");
	};
	let target = mcp_backend.targets.first().expect("remote target");
	let crate::types::agent::McpTargetSpec::Mcp(target_spec) = &target.spec else {
		panic!("expected streamable MCP target");
	};
	assert_eq!(
		target_spec.backend,
		crate::types::agent::SimpleBackendReference::Backend("shared-upstream".into())
	);
	assert_eq!(target_spec.path, "/mcp");
}

#[tokio::test]
async fn test_mcp_named_backend_reference_requires_path_for_mcp() {
	let err = normalize_test_yaml(
		r#"
backends:
- name: shared-upstream
  host: example.com:443
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - mcp:
          targets:
          - name: remote
            mcp:
              backend: shared-upstream
"#,
	)
	.await
	.expect_err("named MCP backend reference should require a path");

	assert!(
		err
			.to_string()
			.contains("path is required when backend is set"),
		"{err}"
	);
}

#[test]
fn test_mcp_backend_host_rejects_mixed_host_and_backend() {
	let err = serde_json::from_value::<super::McpBackendHost>(serde_json::json!({
		"host": "https://example.com/mcp",
		"backend": "shared-upstream"
	}))
	.expect_err("mixed host and backend should be rejected");

	assert!(
		err
			.to_string()
			.contains("cannot mix host/port with backend for MCP target backend configuration"),
		"{err}"
	);
}
