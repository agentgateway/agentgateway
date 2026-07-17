use std::fs;
use std::path::Path;
use std::sync::Arc;

use secrecy::SecretString;

use crate::llm::{AIProvider, NamedAIProvider};
use crate::serdes::FileInlineOrRemote;
use crate::types::agent::{
	Backend, BackendTrafficPolicy, ListenerTarget, PathMatch, PolicyPhase, PolicyTarget, PolicyType,
	ResourceName, RouteBackendTarget, Target, TrafficPolicy,
};
use crate::types::local::NormalizedLocalConfig;
use crate::*;

const TEST_OIDC_JWKS: &str = r#"{"keys":[{"use":"sig","kty":"EC","kid":"kid-1","crv":"P-256","alg":"ES256","x":"WM7udBHga09KxC5kxq6GhrZ9M3Y8S9ZThq_XxsOcDhk","y":"xc7T4afkXmwjEbJMzQXCdQcU3PZKiLFlHl23GE1z4ug"}]}"#;

struct ClearTracingEnv {
	_guard: std::sync::MutexGuard<'static, ()>,
	values: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl ClearTracingEnv {
	fn new() -> Self {
		let guard = crate::config::lock_env_for_tests();
		let keys = [
			"OTLP_ENDPOINT",
			"OTLP_HEADERS",
			"OTLP_PROTOCOL",
			"OTEL_EXPORTER_OTLP_ENDPOINT",
			"OTEL_EXPORTER_OTLP_HEADERS",
			"OTEL_EXPORTER_OTLP_PROTOCOL",
			"OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
			"OTEL_EXPORTER_OTLP_TRACES_HEADERS",
			"OTEL_EXPORTER_OTLP_TRACES_PROTOCOL",
		];
		let values = keys
			.into_iter()
			.map(|key| {
				let value = std::env::var_os(key);
				unsafe {
					std::env::remove_var(key);
				}
				(key, value)
			})
			.collect();
		Self {
			_guard: guard,
			values,
		}
	}
}

impl Drop for ClearTracingEnv {
	fn drop(&mut self) {
		for (key, value) in &self.values {
			match value {
				Some(value) => unsafe {
					std::env::set_var(key, value);
				},
				None => unsafe {
					std::env::remove_var(key);
				},
			}
		}
	}
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
	let resources = crate::resource_manager::ResourceFetcher::direct(test_client());
	super::convert(
		&resources,
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
			gateways: Default::default(),
			routes: vec![],
			tcp_routes: vec![],
			llm: None,
			mcp: None,
			ui: None,
		},
	)
	.await
}

async fn normalize_test_yaml(yaml: &str) -> anyhow::Result<NormalizedLocalConfig> {
	let resources = crate::resource_manager::ResourceFetcher::direct(test_client());
	NormalizedLocalConfig::from(
		&test_config(),
		&resources,
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
	let resources = crate::resource_manager::ResourceFetcher::direct(client);
	let config = crate::config::parse_config(yaml_str.to_string(), None).unwrap();

	NormalizedLocalConfig::from(
		&config,
		&resources,
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

fn selected_ai_provider(normalized: &NormalizedLocalConfig) -> Arc<NamedAIProvider> {
	let backend = normalized
		.backends
		.iter()
		.find(|backend| matches!(backend.backend, Backend::AI(_, _)))
		.expect("expected generated AI backend");
	let Backend::AI(_, ai) = &backend.backend else {
		panic!("expected generated AI backend");
	};
	let (provider, _handle) = ai.select_provider().expect("expected selected provider");
	provider
}

fn assert_hostname_target(target: &Target, expected_host: &str, expected_port: u16) {
	match target {
		Target::Hostname(host, port) => {
			assert_eq!(host.as_str(), expected_host);
			assert_eq!(*port, expected_port);
		},
		other => panic!("expected hostname target, got {other:?}"),
	}
}

#[tokio::test]
async fn test_local_dynamic_backend_reference_uses_generated_backend() {
	let normalized = normalize_test_yaml(
		r#"
binds:
- port: 1080
  listeners:
  - routes:
    - backends:
      - dynamic: {}
"#,
	)
	.await
	.expect("dynamic backend should normalize");

	let route = &normalized.listener_routes[0].1[0];
	let RouteBackendTarget::Backend(backend_key) = &route.backends[0].target else {
		panic!("expected route backend target");
	};
	assert_eq!(
		backend_key,
		"/ns/name/bind/1080/listener0/default/route0/backend0"
	);

	let backend = normalized
		.backends
		.iter()
		.find_map(|backend| match &backend.backend {
			Backend::Dynamic(name, ()) => Some(name),
			_ => None,
		})
		.expect("normalized dynamic backend");
	assert_eq!(
		backend.to_string(),
		"/ns/name/bind/1080/listener0/default/route0/backend0"
	);
}

#[test]
fn test_local_backend_policies_reject_unknown_fields() {
	// serde(flatten) disables deny_unknown_fields on the outer struct, but the
	// flattened SimpleLocalBackendPolicies still rejects leftover unknown keys.
	let err =
		crate::serdes::yamlviajson::from_str::<super::LocalBackendPolicies>("mcpAuthorizatoin: {}")
			.unwrap_err();
	assert!(err.to_string().contains("unknown field"), "{err}");
}

#[tokio::test]
async fn test_multiple_wildcard_binds_rejected() {
	let err = normalize_test_yaml(
		r#"
binds:
- mode: internal
  listeners:
  - routes:
    - backends:
      - dynamic: {}
- mode: internal
  listeners:
  - routes:
    - backends:
      - dynamic: {}
"#,
	)
	.await
	.expect_err("two wildcard binds should be rejected");
	assert!(
		err.to_string().contains("at most one wildcard bind"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_explicit_port_zero_counts_as_wildcard() {
	// `port: 0` on an internal bind is the same wildcard sentinel as omitting the port. A portless
	// wildcard plus an explicit `port: 0` wildcard must still be rejected as two wildcards.
	let err = normalize_test_yaml(
		r#"
binds:
- mode: internal
  listeners:
  - routes:
    - backends:
      - dynamic: {}
- port: 0
  mode: internal
  listeners:
  - routes:
    - backends:
      - dynamic: {}
"#,
	)
	.await
	.expect_err("port: 0 and a portless wildcard are both wildcards and should be rejected");
	assert!(
		err.to_string().contains("at most one wildcard bind"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_single_explicit_port_zero_wildcard_allowed() {
	// A lone `port: 0` internal bind is a valid wildcard and normalizes to the same `bind/wildcard`
	// key as a portless one.
	let normalized = normalize_test_yaml(
		r#"
binds:
- port: 0
  mode: internal
  listeners:
  - routes:
    - backends:
      - dynamic: {}
"#,
	)
	.await
	.expect("a single port: 0 internal bind should normalize");
	assert!(
		normalized.binds.iter().any(|b| b.key == "bind/wildcard"),
		"expected a bind/wildcard key, got: {:?}",
		normalized.binds.iter().map(|b| &b.key).collect::<Vec<_>>()
	);
}

#[tokio::test]
async fn test_single_wildcard_bind_with_exact_internal_bind_allowed() {
	// One exact internal bind (port 443) plus one wildcard internal bind is valid: the exact
	// bind is matched by port, the wildcard serves everything else.
	normalize_test_yaml(
		r#"
binds:
- port: 443
  mode: internal
  listeners:
  - routes:
    - backends:
      - dynamic: {}
- mode: internal
  listeners:
  - routes:
    - backends:
      - dynamic: {}
"#,
	)
	.await
	.expect("one wildcard bind alongside an exact internal bind should normalize");
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
async fn test_llm_provider_reference_config() {
	test_config_parsing("llm_provider_reference").await;
}

#[tokio::test]
async fn test_llm_virtual_model_config() {
	test_config_parsing("llm_virtual_model").await;
}

#[tokio::test]
async fn test_llm_virtual_model_failover_config() {
	test_config_parsing("llm_virtual_model_failover").await;
}

#[tokio::test]
async fn test_llm_virtual_model_conditional_config() {
	test_config_parsing("llm_virtual_model_conditional").await;
}

#[tokio::test]
async fn test_llm_conditional_virtual_model_requires_fallback_last() {
	let err = normalize_test_config(
		r#"
llm:
  models:
  - name: concrete
    provider: openAI
  virtualModels:
  - name: smart
    routing:
      conditional:
        targets:
        - model: concrete
        - when: json(request.body).route == "fast"
          model: concrete
"#,
	)
	.await
	.expect_err("fallback target must be last");
	assert!(
		err
			.to_string()
			.contains("virtual model smart conditional fallback target must be last"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_llm_conditional_virtual_model_rejects_unknown_target() {
	let err = normalize_test_config(
		r#"
llm:
  models:
  - name: concrete
    provider: openAI
  virtualModels:
  - name: smart
    routing:
      conditional:
        targets:
        - model: missing
"#,
	)
	.await
	.expect_err("unknown target should fail");
	assert!(
		err
			.to_string()
			.contains("virtual model target missing does not match any llm.models entry"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_llm_conditional_virtual_model_rejects_unknown_target_before_expression_compile() {
	let err = normalize_test_config(
		r#"
llm:
  models:
  - name: concrete
    provider: openAI
  virtualModels:
  - name: smart
    routing:
      conditional:
        targets:
        - model: concrete
  - name: simple
    routing:
      conditional:
        targets:
        - when: json(request.body).route == "legacy"
          model: concrete
        - model: missing
"#,
	)
	.await
	.expect_err("unknown target should fail before expression compilation");
	assert!(
		err
			.to_string()
			.contains("virtual model target missing does not match any llm.models entry"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_llm_model_rejects_multiple_wildcards() {
	let err = normalize_test_config(
		r#"
llm:
  models:
  - name: '*foo*'
    provider: openAI
"#,
	)
	.await
	.expect_err("model name with multiple wildcards should fail");
	assert!(
		err
			.to_string()
			.contains("model name wildcard may only appear once: '*foo*'"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_llm_model_rejects_middle_wildcard() {
	let err = normalize_test_config(
		r#"
llm:
  models:
  - name: foo*bar
    provider: openAI
"#,
	)
	.await
	.expect_err("model name with middle wildcard should fail");
	assert!(
		err
			.to_string()
			.contains("model name wildcard must be either at the beginning or the end: 'foo*bar'"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_llm_weighted_virtual_model_allows_authorized_target() {
	let normalized = normalize_test_config(
		r#"
llm:
  models:
  - name: concrete
    provider: openAI
    authorization:
      rules:
      - 'request.headers["x-user"] == "admin"'
  virtualModels:
  - name: smart
    routing:
      weighted:
        targets:
        - model: concrete
"#,
	)
	.await
	.expect("weighted target authorization should be preserved on selected target");

	let routes = &normalized.listener_routes[0].1;
	let llm_route = routes
		.iter()
		.find(|route| route.key == "llm:request")
		.expect("expected single LLM request route");
	assert!(
		llm_route.llm_router.is_none(),
		"LLM request route should route through the LLMRouter backend"
	);
	assert!(
		llm_route
			.backends
			.iter()
			.any(|backend| matches!(&backend.target, RouteBackendTarget::Backend(name) if name.as_str() == "/llm:router")),
		"LLM request route should target the LLMRouter backend"
	);
	assert!(
		normalized
			.backends
			.iter()
			.any(|backend| matches!(&backend.backend, Backend::LLMRouter(name, _) if name.name.as_str() == "llm:router")),
		"normalized config should contain the LLMRouter backend"
	);
}

#[tokio::test]
async fn test_llm_conditional_virtual_model_allows_authorized_internal_target() {
	let normalized = normalize_test_config(
		r#"
llm:
  models:
  - name: concrete
    visibility: internal
    provider: openAI
    authorization:
      rules:
      - 'request.headers["x-user"] == "admin"'
  virtualModels:
  - name: smart
    routing:
      conditional:
        targets:
        - model: concrete
"#,
	)
	.await
	.expect("conditional virtual model should preserve target authorization");

	let routes = &normalized.listener_routes[0].1;
	assert!(
		!routes
			.iter()
			.any(|route| route.key.contains("model:concrete")),
		"internal concrete model should not have a direct route"
	);
	let llm_route = routes
		.iter()
		.find(|route| route.key == "llm:request")
		.expect("expected single LLM request route");
	assert!(
		llm_route.llm_router.is_none(),
		"LLM request route should route through the LLMRouter backend"
	);
	assert!(
		llm_route
			.backends
			.iter()
			.any(|backend| matches!(&backend.target, RouteBackendTarget::Backend(name) if name.as_str() == "/llm:router")),
		"LLM request route should target the LLMRouter backend"
	);
	assert!(
		normalized
			.backends
			.iter()
			.any(|backend| matches!(&backend.backend, Backend::LLMRouter(name, _) if name.name.as_str() == "llm:router")),
		"normalized config should contain the LLMRouter backend"
	);
	assert!(
		!routes
			.iter()
			.any(|route| route.key.contains("virtual-model")),
		"virtual model routing should not generate HTTP routes"
	);
}

#[tokio::test]
async fn test_llm_custom_provider_config() {
	let normalized = normalize_test_config(
		r#"
llm:
  models:
  - name: local-custom
    provider:
      custom:
        formats:
        - type: completions
        - type: messages
          path: /api/messages
    params:
      model: upstream-custom
      baseUrl: http://custom.example.com:8080
"#,
	)
	.await
	.expect("custom LLM provider should normalize");

	let provider = selected_ai_provider(&normalized);
	let AIProvider::Custom(custom_provider) = &provider.provider else {
		panic!("expected custom provider");
	};
	assert_eq!(custom_provider.model.as_deref(), Some("upstream-custom"));
	assert!(custom_provider.formats.iter().any(|format| format.format
		== crate::llm::custom::ProviderFormat::Messages
		&& format.path.as_deref() == Some("/api/messages")));
	assert_hostname_target(
		provider
			.host_override
			.as_ref()
			.expect("expected host override"),
		"custom.example.com",
		8080,
	);
}

#[tokio::test]
async fn test_llm_synthetic_provider_defaults_do_not_override_host_override() {
	let normalized = normalize_test_config(
		r#"
llm:
  models:
  - name: hosted-groq
    provider: groq
    params:
      hostOverride: proxy.example.com:8443
      pathPrefix: /proxy/openai
"#,
	)
	.await
	.expect("synthetic LLM provider should normalize with explicit host override");

	let provider = selected_ai_provider(&normalized);
	assert!(matches!(provider.provider, AIProvider::Custom(_)));
	assert_hostname_target(
		provider
			.host_override
			.as_ref()
			.expect("expected host override"),
		"proxy.example.com",
		8443,
	);
	assert_eq!(provider.path_prefix.as_deref(), Some("/proxy/openai"));
}

#[tokio::test]
async fn test_llm_base_url_does_not_override_explicit_path_prefix() {
	let normalized = normalize_test_config(
		r#"
llm:
  models:
  - name: proxied-groq
    provider: groq
    params:
      baseUrl: https://api.groq.com/openai/v1
      pathPrefix: /gateway/openai
"#,
	)
	.await
	.expect("synthetic LLM provider should normalize with explicit path prefix");

	let provider = selected_ai_provider(&normalized);
	assert!(matches!(provider.provider, AIProvider::Custom(_)));
	assert_hostname_target(
		provider
			.host_override
			.as_ref()
			.expect("expected host override"),
		"api.groq.com",
		443,
	);
	assert_eq!(provider.path_prefix.as_deref(), Some("/gateway/openai"));
}

#[tokio::test]
async fn test_llm_base_url_does_not_override_explicit_host_override() {
	let normalized = normalize_test_config(
		r#"
llm:
  models:
  - name: proxied-fireworks
    provider: fireworks
    params:
      baseUrl: https://api.fireworks.ai/inference/v1
      hostOverride: proxy.example.com:8443
"#,
	)
	.await
	.expect("synthetic LLM provider should normalize with explicit host override");

	let provider = selected_ai_provider(&normalized);
	assert!(matches!(provider.provider, AIProvider::Custom(_)));
	assert_hostname_target(
		provider
			.host_override
			.as_ref()
			.expect("expected host override"),
		"proxy.example.com",
		8443,
	);
	assert_eq!(provider.path_prefix.as_deref(), Some("/inference/v1"));
}

#[tokio::test]
async fn test_mcp_simple_config() {
	test_config_parsing("mcp_simple").await;
}

#[tokio::test]
async fn test_llm_mcp_same_port_share_listener_routes() {
	let normalized = normalize_test_yaml(
		r#"
llm:
  port: 3000
  models:
  - name: gpt-4
    provider: openAI
mcp:
  targets:
  - name: time
    stdio:
      cmd: uvx
"#,
	)
	.await
	.expect("same-port LLM and MCP should normalize");

	assert_eq!(normalized.binds.len(), 1);
	assert_eq!(normalized.binds[0].address.port(), 3000);
	assert_eq!(
		normalized.binds[0]
			.listeners
			.iter()
			.map(|listener| listener.key.as_str())
			.collect::<Vec<_>>(),
		vec!["llm"],
	);
	assert_eq!(normalized.listener_routes.len(), 1);
	assert_eq!(normalized.listener_routes[0].0.as_str(), "llm");
	let routes = &normalized.listener_routes[0].1;
	assert!(
		routes
			.iter()
			.any(|route| route.key.as_str() == "llm:request")
	);
	let mcp_route = routes
		.iter()
		.find(|route| route.key.as_str() == "mcp:default")
		.expect("expected MCP route on shared listener");
	assert_eq!(
		mcp_route
			.matches
			.iter()
			.map(|route_match| match &route_match.path {
				PathMatch::PathPrefix(path) => path.as_str(),
				other => panic!("expected path prefix match, got {other:?}"),
			})
			.collect::<Vec<_>>(),
		vec!["/mcp", "/sse", "/.well-known"],
	);
}

#[tokio::test]
async fn test_llm_mcp_same_port_rejects_llm_tls() {
	let err = normalize_test_yaml(
		r#"
llm:
  port: 3000
  tls:
    cert: inline
    key: inline
  models:
  - name: gpt-4
    provider: openAI
mcp:
  targets:
  - name: time
    stdio:
      cmd: uvx
"#,
	)
	.await
	.expect_err("same-port LLM and MCP should reject llm.tls");
	assert!(
		err
			.to_string()
			.contains("top-level llm and mcp cannot share a port when llm.tls is configured"),
		"{err:?}"
	);
}

#[tokio::test]
async fn test_gateways_attach_llm_mcp_and_ui_to_one_listener() {
	let normalized = normalize_test_yaml(&format!(
		r#"
gateways:
  public:
    port: 3000
llm:
  gateways: public
  models:
  - name: gpt-4
    provider: openAI
mcp:
  gateways: [public]
  targets:
  - name: time
    stdio:
      cmd: uvx
ui:
  gateways: [public]
  policies:
    oidc:
      issuer: https://issuer.example.com
      authorizationEndpoint: https://issuer.example.com/authorize
      tokenEndpoint: https://issuer.example.com/token
      jwks: '{TEST_OIDC_JWKS}'
      clientId: client-id
      clientSecret: client-secret
      redirectURI: http://localhost:3000/oauth/callback
"#,
	))
	.await
	.expect("gateway-attached LLM, MCP, and UI should normalize");

	assert_eq!(normalized.binds.len(), 1);
	assert_eq!(normalized.binds[0].address.port(), 3000);
	assert_eq!(
		normalized.binds[0]
			.listeners
			.iter()
			.map(|listener| listener.key.as_str())
			.collect::<Vec<_>>(),
		vec!["gateway/public"],
	);

	let routes = normalized
		.listener_routes
		.iter()
		.find(|(listener, _)| listener.as_str() == "gateway/public")
		.map(|(_, routes)| routes)
		.expect("public gateway listener routes");
	assert!(
		routes
			.iter()
			.any(|route| route.key.as_str() == "gateway/public/llm:request")
	);
	let mcp_route = routes
		.iter()
		.find(|route| route.key.as_str() == "gateway/public/mcp:default")
		.expect("MCP route");
	assert_eq!(
		mcp_route
			.matches
			.iter()
			.map(|route_match| match &route_match.path {
				PathMatch::PathPrefix(path) => path.as_str(),
				other => panic!("expected path prefix match, got {other:?}"),
			})
			.collect::<Vec<_>>(),
		vec!["/mcp", "/sse", "/.well-known"],
	);
	let ui_route = routes
		.iter()
		.find(|route| route.key.as_str() == "gateway/public/ui")
		.expect("UI route");
	assert!(ui_route.matches.iter().any(
		|route_match| matches!(&route_match.path, PathMatch::Exact(path) if path.as_str() == "/")
	));
	assert!(ui_route.matches.iter().any(
		|route_match| matches!(&route_match.path, PathMatch::PathPrefix(path) if path.as_str() == "/ui")
	));
	assert!(ui_route.matches.iter().any(
		|route_match| matches!(&route_match.path, PathMatch::Exact(path) if path.as_str() == "/oauth/callback")
	));
	assert!(
		ui_route
			.inline_policies
			.iter()
			.any(|policy| matches!(policy, TrafficPolicy::Oidc(_)))
	);
	assert!(normalized.backends.iter().any(|backend| {
		matches!(&backend.backend, Backend::Internal(name, _) if name.name.as_str() == "ui")
	}));
}

#[tokio::test]
async fn test_default_gateway_is_implicit_for_attached_surfaces_and_routes() {
	let normalized = normalize_test_yaml(&format!(
		r#"
gateways:
  default:
    port: 3000
routes:
- name: app
  matches:
  - path:
      exact: /app
  backends:
  - host: example.com:80
llm:
  models:
  - name: gpt-4
    provider: openAI
mcp:
  targets:
  - name: time
    stdio:
      cmd: uvx
ui:
  policies:
    oidc:
      issuer: https://issuer.example.com
      authorizationEndpoint: https://issuer.example.com/authorize
      tokenEndpoint: https://issuer.example.com/token
      jwks: '{TEST_OIDC_JWKS}'
      clientId: client-id
      clientSecret: client-secret
      redirectURI: http://localhost:3000/oauth/callback
"#,
	))
	.await
	.expect("default gateway should be implicit");

	assert_eq!(normalized.binds.len(), 1);
	assert_eq!(normalized.binds[0].address.port(), 3000);
	let routes = normalized
		.listener_routes
		.iter()
		.find(|(listener, _)| listener.as_str() == "gateway/default")
		.map(|(_, routes)| routes)
		.expect("default gateway listener routes");
	assert!(
		routes
			.iter()
			.any(|route| route.key.as_str() == "gateway/default/default/app")
	);
	assert!(
		routes
			.iter()
			.any(|route| route.key.as_str() == "gateway/default/llm:request")
	);
	assert!(
		routes
			.iter()
			.any(|route| route.key.as_str() == "gateway/default/mcp:default")
	);
	assert!(
		routes
			.iter()
			.any(|route| route.key.as_str() == "gateway/default/ui")
	);
}

#[tokio::test]
async fn test_explicit_llm_mcp_ports_take_precedence_over_default_gateway() {
	let normalized = normalize_test_yaml(
		r#"
gateways:
  default:
    port: 8080
llm:
  port: 4000
  models:
  - name: gpt-4
    provider: openAI
mcp:
  port: 3000
  targets:
  - name: time
    stdio:
      cmd: uvx
"#,
	)
	.await
	.expect("explicit LLM/MCP ports should stay port-bound");

	assert_eq!(
		normalized
			.binds
			.iter()
			.map(|bind| bind.address.port())
			.collect::<Vec<_>>(),
		vec![8080, 4000, 3000],
	);
	assert!(normalized.listener_routes.iter().any(|(listener, routes)| {
		listener.as_str() == "llm"
			&& routes
				.iter()
				.any(|route| route.key.as_str() == "llm:request")
	}));
	assert!(normalized.listener_routes.iter().any(|(listener, routes)| {
		listener.as_str() == "mcp"
			&& routes
				.iter()
				.any(|route| route.key.as_str() == "mcp:default")
	}));
}

#[tokio::test]
async fn test_llm_gateways_preserves_route_policies() {
	let normalized = normalize_test_yaml(
		r#"
gateways:
  public:
    port: 3000
llm:
  gateways: [public]
  policies:
    authorization:
      rules:
      - 'request.path == "/"'
  models:
  - name: gpt-4
    provider: openAI
"#,
	)
	.await
	.expect("llm.gateways should allow route-level llm.policies");

	let llm_route = normalized
		.listener_routes
		.iter()
		.find(|(listener, _)| listener.as_str() == "gateway/public")
		.and_then(|(_, routes)| {
			routes
				.iter()
				.find(|route| route.key.as_str() == "gateway/public/llm:request")
		})
		.expect("gateway-attached LLM route");
	assert!(
		llm_route
			.inline_policies
			.iter()
			.any(|policy| matches!(policy, TrafficPolicy::Authorization(_)))
	);
}

#[tokio::test]
async fn test_ui_policies_rejects_non_ui_policy() {
	for field in ["ai", "requestHeaderModifier", "responseHeaderModifier"] {
		let err = normalize_test_yaml(&format!(
			r#"
gateways:
  public:
    port: 3000
ui:
  gateways: [public]
  policies:
    {field}: {{}}
"#,
		))
		.await
		.expect_err("ui.policies should reject non-UI policies");

		assert!(
			err
				.to_string()
				.contains(&format!("unknown field `{field}`")),
			"{err:?}"
		);
	}
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
async fn test_local_mcp_target_name_wiring_rejects_underscore() {
	let yaml = r#"
mcp:
  targets:
  - name: "bad_name"
    stdio:
      cmd: echo
"#;
	normalize_test_yaml(yaml)
		.await
		.expect_err("MCP target name containing '_' should be rejected");
}

#[tokio::test]
async fn test_local_mcp_stdio_target_rejects_policies() {
	let yaml = r#"
mcp:
  targets:
  - name: everything
    stdio:
      cmd: echo
    policies:
      backendTLS: {}
"#;
	normalize_test_yaml(yaml)
		.await
		.expect_err("policies on a stdio MCP target should be rejected");
}

#[tokio::test]
async fn test_local_mcp_target_rejects_mcp_policies() {
	let guardrails_yaml = r#"
mcp:
  targets:
  - name: everything
    mcp:
      host: localhost:8080
    policies:
      mcpGuardrails:
        processors: []
"#;
	let err = normalize_test_yaml(guardrails_yaml)
		.await
		.expect_err("mcpGuardrails on an MCP target should be rejected");
	assert!(
		err.to_string().contains("mcpGuardrails"),
		"error should mention mcpGuardrails, got: {err}"
	);

	let authorization_yaml = r#"
mcp:
  targets:
  - name: everything
    mcp:
      host: localhost:8080
    policies:
      mcpAuthorization:
        rules:
        - 'mcp.tool.name == "echo"'
"#;
	let err = normalize_test_yaml(authorization_yaml)
		.await
		.expect_err("mcpAuthorization on an MCP target should be rejected");
	assert!(
		err.to_string().contains("mcpAuthorization"),
		"error should mention mcpAuthorization, got: {err}"
	);
}

#[tokio::test]
async fn test_aws_config() {
	test_config_parsing("aws").await;
}

#[tokio::test]
async fn test_gateway_config() {
	test_config_parsing("gateway").await;
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
            destinationMode: passthrough
"#;

	normalize_test_config(input)
		.await
		.expect("service backends should allow inference routing");
}

#[tokio::test]
async fn test_local_ext_authz_conditional_policy() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - policies:
        extAuthz:
          conditional:
          - condition: request.path == "/admin"
            host: 127.0.0.1:9000
          - host: 127.0.0.1:9001
      backends:
      - host: 127.0.0.1:8000
"#;

	let normalized = normalize_test_yaml(input).await.unwrap();
	let route = &normalized.listener_routes[0].1[0];
	let Some(TrafficPolicy::ExtAuthz(ext_authz)) = route
		.inline_policies
		.iter()
		.find(|policy| matches!(policy, TrafficPolicy::ExtAuthz(_)))
	else {
		panic!("expected extAuthz policy");
	};
	let entries = ext_authz.iter().collect::<Vec<_>>();
	assert_eq!(entries.len(), 2);
	assert_eq!(
		entries[0].condition.as_ref().unwrap().original_expression,
		"request.path == \"/admin\""
	);
	assert!(entries[1].condition.is_none());
}

#[tokio::test]
async fn test_local_ext_authz_http_include_response_headers() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - policies:
        extAuthz:
          host: 127.0.0.1:9000
          protocol:
            http:
              includeResponseHeaders:
              - x-auth-request-user
      backends:
      - host: 127.0.0.1:8000
"#;

	normalize_test_yaml(input)
		.await
		.expect("http extAuthz includeResponseHeaders should accept header names");
}

#[tokio::test]
async fn test_local_backend_ext_authz_policy() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - host: 127.0.0.1:8000
        policies:
          extAuthz:
            host: 127.0.0.1:9000
      - host: 127.0.0.1:8001
"#;

	let normalized = normalize_test_yaml(input).await.unwrap();
	let route = &normalized.listener_routes[0].1[0];
	assert!(route.inline_policies.is_empty());
	assert!(matches!(
		route.backends[0].inline_policies.as_slice(),
		[BackendTrafficPolicy::ExtAuthz(_)]
	));
	assert!(route.backends[1].inline_policies.is_empty());
}

#[tokio::test]
async fn test_local_ext_authz_conditional_fallback_must_be_last() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - policies:
        extAuthz:
          conditional:
          - host: 127.0.0.1:9000
          - condition: request.path == "/admin"
            host: 127.0.0.1:9001
      backends:
      - host: 127.0.0.1:8000
"#;

	let err = normalize_test_yaml(input).await.unwrap_err();
	assert!(
		err
			.to_string()
			.contains("conditional policy entries without condition must be last"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn test_local_ext_authz_conditional_reports_field_errors() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - policies:
        extAuthz:
          conditional:
          - condition: 1
            host: 127.0.0.1:9000
      backends:
      - host: 127.0.0.1:8000
"#;

	let err = normalize_test_yaml(input).await.unwrap_err();
	assert!(
		err.to_string().contains("invalid type: integer `1`"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn test_local_conditional_policy_types() {
	let input = r#"
binds:
- port: 3000
  listeners:
  - routes:
    - policies:
        directResponse:
          conditional:
          - condition: request.path == "/direct"
            status: 403
            body: denied
          - status: 404
            body: missing
        transformations:
          conditional:
          - condition: request.path == "/transform"
            request:
              set:
                x-test: "'transform'"
          - response:
              add:
                x-fallback: "'true'"
        extProc:
          conditional:
          - condition: request.path == "/process"
            host: 127.0.0.1:9000
          - host: 127.0.0.1:9001
        localRateLimit:
          conditional:
          - condition: request.path == "/local-limit"
            maxTokens: 10
            tokensPerFill: 1
            fillInterval: 1s
          - maxTokens: 1
            tokensPerFill: 1
            fillInterval: 60s
        remoteRateLimit:
          conditional:
          - condition: request.path == "/remote-limit"
            domain: agentgateway
            host: 127.0.0.1:9002
            descriptors:
            - entries:
              - key: user
                value: '"test-user"'
          - domain: fallback
            host: 127.0.0.1:9003
            descriptors:
            - entries:
              - key: user
                value: '"fallback"'
        buffer:
          request:
            maxBytes: 10
          response:
            maxBytes: 20
      backends:
      - host: 127.0.0.1:8000
"#;

	let normalized = normalize_test_yaml(input).await.unwrap();
	let route = &normalized.listener_routes[0].1[0];

	for policy in [
		"directResponse",
		"transformation",
		"extProc",
		"localRateLimit",
		"remoteRateLimit",
	] {
		let (len, fallback_is_none) = route
			.inline_policies
			.iter()
			.find_map(|p| match (policy, p) {
				("directResponse", TrafficPolicy::DirectResponse(p)) => {
					let entries = p.iter().collect::<Vec<_>>();
					Some((entries.len(), entries[1].condition.is_none()))
				},
				("transformation", TrafficPolicy::Transformation(p)) => {
					let entries = p.iter().collect::<Vec<_>>();
					Some((entries.len(), entries[1].condition.is_none()))
				},
				("extProc", TrafficPolicy::ExtProc(p)) => {
					let entries = p.iter().collect::<Vec<_>>();
					Some((entries.len(), entries[1].condition.is_none()))
				},
				("localRateLimit", TrafficPolicy::LocalRateLimit(p)) => {
					let entries = p.iter().collect::<Vec<_>>();
					Some((entries.len(), entries[1].condition.is_none()))
				},
				("remoteRateLimit", TrafficPolicy::RemoteRateLimit(p)) => {
					let entries = p.iter().collect::<Vec<_>>();
					Some((entries.len(), entries[1].condition.is_none()))
				},
				_ => None,
			})
			.unwrap_or_else(|| panic!("expected {policy} policy"));

		assert_eq!(len, 2, "expected two {policy} entries");
		assert!(fallback_is_none, "expected {policy} fallback condition");
	}

	let buffer = route
		.inline_policies
		.iter()
		.find_map(|p| match p {
			TrafficPolicy::Buffer(p) => Some(p.iter().next().expect("buffer policy entry").pol.as_ref()),
			_ => None,
		})
		.expect("expected buffer policy");
	assert_eq!(buffer.request.as_ref().unwrap().max_bytes, Some(10));
	assert_eq!(buffer.response.as_ref().unwrap().max_bytes, Some(20));
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
fn test_migrate_deprecated_local_config_moves_fields() {
	let _env = ClearTracingEnv::new();
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
fn test_migrate_deprecated_tracing_https_endpoint_adds_backend_tls() {
	let _env = ClearTracingEnv::new();
	let input = r#"
config:
  tracing:
    otlpEndpoint: https://tracing.example.com:4318
    otlpProtocol: http
"#;
	let out = super::migrate_deprecated_local_config(input).unwrap();
	let v: serde_json::Value = crate::serdes::yamlviajson::from_str(&out).unwrap();
	let tracing = v.get("frontendPolicies").unwrap().get("tracing").unwrap();
	let policies = tracing
		.get("policies")
		.and_then(serde_json::Value::as_array)
		.expect("https tracing endpoint should add backend TLS policy");
	assert!(
		policies.iter().any(|policy| {
			policy
				.get("backendTLS")
				.and_then(|tls| tls.get("systemRoots"))
				.and_then(serde_json::Value::as_bool)
				.unwrap_or(false)
		}),
		"https tracing endpoint should use system root TLS"
	);
}

#[test]
fn test_migrate_deprecated_tracing_https_endpoint_uses_default_port_and_path() {
	let _env = ClearTracingEnv::new();
	let input = r#"
config:
  tracing:
    otlpEndpoint: https://tracing.example.com/api/public/otel/v1/traces
    otlpProtocol: http
"#;
	let out = super::migrate_deprecated_local_config(input).unwrap();
	let v: serde_json::Value = crate::serdes::yamlviajson::from_str(&out).unwrap();
	let tracing = v.get("frontendPolicies").unwrap().get("tracing").unwrap();
	assert_eq!(
		tracing.get("inlineBackend").unwrap(),
		"tracing.example.com:443"
	);
	assert_eq!(tracing.get("path").unwrap(), "/api/public/otel/v1/traces");
}

#[rstest::rstest]
#[case::https_scheme("https://tracing.example.com:4318", "http", "tracing.example.com:4318")]
#[case::http_scheme("http://tracing.example.com:4318", "http", "tracing.example.com:4318")]
#[case::grpc_scheme("grpc://tracing.example.com:4317", "grpc", "tracing.example.com:4317")]
#[case::no_scheme("tracing.example.com:4317", "grpc", "tracing.example.com:4317")]
fn test_deprecated_tracing_endpoint_schemes(
	#[case] endpoint: &str,
	#[case] protocol: &str,
	#[case] expected: &str,
) {
	let _env = ClearTracingEnv::new();
	let input =
		format!("config:\n  tracing:\n    otlpEndpoint: {endpoint}\n    otlpProtocol: {protocol}\n");
	let out = super::migrate_deprecated_local_config(&input).unwrap();
	let v: serde_json::Value = crate::serdes::yamlviajson::from_str(&out).unwrap();
	let tracing = v.get("frontendPolicies").unwrap().get("tracing").unwrap();
	assert_eq!(tracing.get("inlineBackend").unwrap(), expected);
}

#[rstest::rstest]
#[case::unrecognized_scheme("nateisgreat://tracing.example.com:4317")]
fn test_deprecated_tracing_endpoint_unrecognized_scheme_error(#[case] endpoint: &str) {
	let _env = ClearTracingEnv::new();
	let input =
		format!("config:\n  tracing:\n    otlpEndpoint: {endpoint}\n    otlpProtocol: grpc\n");
	let err = super::migrate_deprecated_local_config(&input)
		.unwrap_err()
		.to_string();
	assert!(
		err.contains("tracing"),
		"error message should mention 'tracing': {err}"
	);
	assert!(
		err.contains("failed"),
		"error message should mention 'failed': {err}"
	);
	assert!(
		err.contains(endpoint),
		"error message should include the invalid endpoint: {err}"
	);
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

#[tokio::test]
async fn test_top_level_backend_targeted_transformations_are_backend_policies() {
	let normalized = normalize_test_yaml(
		r#"
backends:
- name: upstream-1
  host: 127.0.0.1:8000
policies:
- name:
    name: add-header
    namespace: ""
  target:
    backend:
      backend:
        name: upstream-1
        namespace: ""
  policy:
    transformations:
      request:
        set:
          x-test: "'backend'"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - backend: upstream-1
"#,
	)
	.await
	.expect("backend-targeted transformations should normalize");

	let policy = normalized
		.policies
		.iter()
		.find(|policy| {
			policy
				.name
				.as_ref()
				.is_some_and(|name| name.name.as_str() == "add-header")
		})
		.expect("expected top-level add-header policy");

	assert!(
		matches!(policy.target, PolicyTarget::Backend(_)),
		"expected backend target, got {:?}",
		policy.target
	);
	assert!(
		matches!(
			&policy.policy,
			PolicyType::Backend(BackendTrafficPolicy::Transformation(_))
		),
		"expected BackendTrafficPolicy::Transformation, got {:?}",
		policy.policy
	);
	assert!(
		policy.policy.as_backend().is_some(),
		"backend-targeted transformations must surface via as_backend()"
	);
}

#[tokio::test]
async fn test_top_level_backend_targeted_conditional_transformations_are_rejected() {
	let err = normalize_test_yaml(
		r#"
backends:
- name: upstream-1
  host: 127.0.0.1:8000
policies:
- name:
    name: add-header
    namespace: ""
  target:
    backend:
      backend:
        name: upstream-1
        namespace: ""
  policy:
    transformations:
      conditional:
      - request:
          set:
            x-test: "'backend'"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - backend: upstream-1
"#,
	)
	.await
	.expect_err("conditional transformations on backend targets should fail");

	assert!(
		err
			.to_string()
			.contains("conditional transformations are not supported on backend-targeted policies"),
		"unexpected error: {err}"
	);
}

#[tokio::test]
async fn test_oauth_token_exchange_reports_validation_errors_from_local_config() {
	let err = normalize_test_yaml(
		r#"
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - host: 127.0.0.1:8080
        policies:
          backendAuth:
            oauthTokenExchange:
              host: 127.0.0.1:9000
              clientAuth:
                clientId: gateway-client
                clientSecret: ""
"#,
	)
	.await
	.expect_err("empty client secret should fail at config load");

	assert!(
		err.to_string().contains("client_secret"),
		"returned unexpected error: {err}"
	);
}
