use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::types::agent::{HeaderValueMatch, PolicyPhase, PolicyType, TrafficPolicy};
use crate::types::local::NormalizedLocalConfig;
use crate::*;

async fn test_config_parsing(test_name: &str) {
	// Make it static
	super::STARTUP_TIMESTAMP.get_or_init(|| 0);
	let test_dir = Path::new("src/types/local_tests");
	let input_path = test_dir.join(format!("{}_config.yaml", test_name));

	let yaml_str = fs::read_to_string(&input_path).unwrap();

	// Create a test client. Ideally we could have a fake one
	let client = client::Client::new(
		&client::Config {
			resolver_cfg: hickory_resolver::config::ResolverConfig::default(),
			resolver_opts: hickory_resolver::config::ResolverOpts::default(),
		},
		None,
		BackendConfig::default(),
		None,
	);
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

fn parse_test_config() -> crate::Config {
	crate::config::parse_config("{}".to_string(), None).unwrap()
}

fn parse_test_config_with_oidc() -> crate::Config {
	static OIDC_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
	let _guard = OIDC_ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
	unsafe {
		env::set_var(
			"OIDC_COOKIE_SECRET",
			"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		);
	}
	let config = parse_test_config();
	unsafe {
		env::remove_var("OIDC_COOKIE_SECRET");
	}
	config
}

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

fn test_listener_target() -> ListenerTarget {
	ListenerTarget {
		gateway_name: "name".into(),
		gateway_namespace: "ns".into(),
		listener_name: None,
	}
}

async fn normalize_test_yaml(
	config: &crate::Config,
	yaml: &str,
) -> anyhow::Result<NormalizedLocalConfig> {
	NormalizedLocalConfig::from(config, test_client(), test_listener_target(), yaml).await
}

async fn normalize_test_yaml_with_oidc(yaml: &str) -> anyhow::Result<NormalizedLocalConfig> {
	let config = parse_test_config_with_oidc();
	normalize_test_yaml(&config, yaml).await
}

fn listener_for_tests(normalized: &NormalizedLocalConfig) -> &crate::types::agent::Listener {
	normalized
		.binds
		.first()
		.expect("bind")
		.listeners
		.iter()
		.next()
		.expect("listener")
}

fn route_for_tests<'a>(
	listener: &'a crate::types::agent::Listener,
	name: &str,
) -> &'a std::sync::Arc<crate::types::agent::Route> {
	listener
		.routes
		.iter()
		.find(|route| route.name.name.as_str() == name)
		.expect("route")
}

fn effective_oidc_policy_for_route(
	config: &crate::Config,
	normalized: &NormalizedLocalConfig,
	route_name: &str,
) -> crate::http::oidc::OidcPolicy {
	let listener = listener_for_tests(normalized);
	let route = route_for_tests(listener, route_name);
	let stores = super::build_oidc_validation_stores(config, normalized);
	stores
		.read_binds()
		.route_policies(
			&crate::store::RoutePath {
				listener: &listener.name,
				route: &route.name,
			},
			&route.inline_policies,
		)
		.oidc
		.expect("effective oidc policy")
}

fn route_phase_targeted_oidc_policy(
	normalized: &NormalizedLocalConfig,
) -> (
	&crate::types::agent::PolicyKey,
	&crate::http::oidc::OidcPolicy,
) {
	normalized
		.policies
		.iter()
		.find_map(|policy| match &policy.policy {
			PolicyType::Traffic(phased)
				if phased.phase == PolicyPhase::Route
					&& matches!(&phased.policy, TrafficPolicy::Oidc(_)) =>
			{
				let TrafficPolicy::Oidc(oidc) = &phased.policy else {
					unreachable!("matched above")
				};
				Some((&policy.key, oidc))
			},
			_ => None,
		})
		.expect("targeted oidc policy")
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

#[tokio::test]
async fn test_oidc_missing_callback_route_is_rejected() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: application
      matches:
      - path:
          exact: /app
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
"#;

	let config = parse_test_config_with_oidc();

	let err = normalize_test_yaml(&config, yaml)
		.await
		.expect_err("missing callback route should fail validation");
	assert!(err.to_string().contains("no route can own that callback"));
}

#[tokio::test]
async fn test_oidc_exact_callback_route_shadowed_by_different_policy_is_rejected() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: callback-a
      matches:
      - path:
          exact: /oauth/callback
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: client-a
          clientSecret: secret-a
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
    - name: callback-b
      matches:
      - path:
          exact: /oauth/callback
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: client-b
          clientSecret: secret-b
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
"#;

	let err = normalize_test_yaml_with_oidc(yaml)
		.await
		.expect_err("different exact callback owners should fail validation");
	assert!(
		err
			.to_string()
			.contains("wins callback selection with a different effective oidc policy")
	);
}

#[tokio::test]
async fn test_listener_scoped_oidc_policy_is_applied_at_route_phase() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    policies:
      oidc:
        issuer: https://issuer.example.com
        authorizationEndpoint: https://issuer.example.com/authorize
        tokenEndpoint: https://issuer.example.com/token
        jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
        clientId: agentgateway-browser
        clientSecret: agentgateway-secret
        redirectURI: http://localhost:3000/oauth/callback
    routes:
    - name: callback
      matches:
      - path:
          exact: /oauth/callback
      backends:
      - host: localhost:18080
    - name: application
      matches:
      - path:
          pathPrefix: /
      backends:
      - host: localhost:18080
"#;

	normalize_test_yaml_with_oidc(yaml)
		.await
		.expect("listener-scoped oidc should normalize successfully");
}

#[tokio::test]
async fn test_inline_oidc_policy_identity_is_keyed_by_callback_owner_route() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: callback
      matches:
      - path:
          exact: /oauth/callback
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
    - name: application
      matches:
      - path:
          pathPrefix: /
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
"#;

	let config = parse_test_config_with_oidc();
	let normalized = normalize_test_yaml(&config, yaml)
		.await
		.expect("inline oidc should normalize successfully");
	let listener = listener_for_tests(&normalized);
	let callback_route = route_for_tests(listener, "callback");
	let callback_policy = effective_oidc_policy_for_route(&config, &normalized, "callback");
	let application_policy = effective_oidc_policy_for_route(&config, &normalized, "application");
	let expected_policy_id =
		crate::http::oidc::PolicyId::from(format!("route/{}", callback_route.key));

	assert_eq!(callback_policy.policy_id, expected_policy_id);
	assert_eq!(application_policy.policy_id, expected_policy_id);
	assert_eq!(
		callback_policy.session.cookie_name,
		application_policy.session.cookie_name
	);
	assert_eq!(
		callback_policy.session.transaction_cookie_name,
		application_policy.session.transaction_cookie_name
	);
}

#[tokio::test]
async fn test_listener_scoped_oidc_policy_identity_is_keyed_by_targeted_policy() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    policies:
      oidc:
        issuer: https://issuer.example.com
        authorizationEndpoint: https://issuer.example.com/authorize
        tokenEndpoint: https://issuer.example.com/token
        jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
        clientId: agentgateway-browser
        clientSecret: agentgateway-secret
        redirectURI: http://localhost:3000/oauth/callback
    routes:
    - name: callback
      matches:
      - path:
          exact: /oauth/callback
      backends:
      - host: localhost:18080
    - name: application
      matches:
      - path:
          pathPrefix: /
      backends:
      - host: localhost:18080
"#;

	let config = parse_test_config_with_oidc();
	let normalized = normalize_test_yaml(&config, yaml)
		.await
		.expect("listener-scoped oidc should normalize successfully");
	let (policy_key, targeted_oidc) = route_phase_targeted_oidc_policy(&normalized);
	let callback_policy = effective_oidc_policy_for_route(&config, &normalized, "callback");
	let application_policy = effective_oidc_policy_for_route(&config, &normalized, "application");
	let expected_policy_id = crate::http::oidc::PolicyId::from(format!("policy/{policy_key}"));

	assert_eq!(targeted_oidc.policy_id, expected_policy_id);
	assert_eq!(callback_policy.policy_id, expected_policy_id);
	assert_eq!(application_policy.policy_id, expected_policy_id);
}

#[tokio::test]
async fn test_oidc_callback_path_prefix_route_does_not_count_as_owner() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: application
      matches:
      - path:
          pathPrefix: /
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
"#;

	let err = normalize_test_yaml_with_oidc(yaml)
		.await
		.expect_err("prefix route should not count as callback owner");
	assert!(
		err
			.to_string()
			.contains("callback ownership requires an exact callback route")
	);
}

#[tokio::test]
async fn test_oidc_callback_regex_route_does_not_count_as_owner() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: application
      matches:
      - path:
          regex: ^/oauth/callback$
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
"#;

	let err = normalize_test_yaml_with_oidc(yaml)
		.await
		.expect_err("regex route should not count as callback owner");
	assert!(
		err
			.to_string()
			.contains("callback ownership requires an exact callback route")
	);
}

#[tokio::test]
async fn test_oidc_exact_callback_route_shadowed_by_non_oidc_route_is_rejected() {
	let yaml = r#"
binds:
- port: 3000
  listeners:
  - name: default
    protocol: HTTP
    routes:
    - name: callback
      matches:
      - path:
          exact: /oauth/callback
      backends:
      - host: localhost:18080
    - name: application
      matches:
      - path:
          pathPrefix: /
      policies:
        oidc:
          issuer: https://issuer.example.com
          authorizationEndpoint: https://issuer.example.com/authorize
          tokenEndpoint: https://issuer.example.com/token
          jwks: '{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}'
          clientId: agentgateway-browser
          clientSecret: agentgateway-secret
          redirectURI: http://localhost:3000/oauth/callback
      backends:
      - host: localhost:18080
"#;

	let err = normalize_test_yaml_with_oidc(yaml)
		.await
		.expect_err("non-oidc exact callback route should reject shadowed oidc callback");
	assert!(
		err
			.to_string()
			.contains("wins callback selection without oidc")
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
