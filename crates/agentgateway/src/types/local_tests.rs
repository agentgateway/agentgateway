use std::fs;
use std::path::Path;

use crate::types::agent::HeaderValueMatch;
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
