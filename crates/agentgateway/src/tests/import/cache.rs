use crate::import::{ImportFinding, ImportResult, ImportStatus, import_config};

const INPUT: &str = include_str!("cache-input.yaml");
const MODEL: &str =
	"model_list:\n- model_name: chat\n  litellm_params:\n    model: openai/gpt-4o\n";

fn import_settings(settings: &str) -> ImportResult {
	let result = import_config("litellm", &format!("{MODEL}{settings}"))
		.expect("cache settings should not prevent importing a valid model");
	let baseline = import_config("litellm", MODEL).unwrap();
	assert_eq!(
		result.config, baseline.config,
		"cache settings changed routing"
	);
	result
}

fn finding<'a>(result: &'a ImportResult, path: &str, status: ImportStatus) -> &'a ImportFinding {
	let matches = result
		.findings
		.iter()
		.filter(|finding| finding.source_path == path)
		.collect::<Vec<_>>();
	assert_eq!(
		matches.len(),
		1,
		"expected one migration finding for {path}"
	);
	assert_eq!(matches[0].status, status, "unexpected status for {path}");
	matches[0]
}

fn assert_concepts(finding: &ImportFinding, concepts: &[&str]) {
	let message = finding.message.to_lowercase();
	for concept in concepts {
		assert!(
			message.contains(concept),
			"{} does not explain {concept:?}: {}",
			finding.source_path,
			finding.message
		);
	}
}

#[test]
fn response_cache_migration_reports_actionable_source_fields() {
	let result = import_config("litellm", INPUT).expect("valid source configuration");
	for path in [
		"litellm_settings.cache",
		"litellm_settings.cache_params.type",
		"litellm_settings.cache_params.host",
		"litellm_settings.cache_params.port",
		"litellm_settings.cache_params.password",
		"litellm_settings.cache_params.ttl",
		"litellm_settings.cache_params.namespace",
	] {
		finding(&result, path, ImportStatus::Manual);
	}
	assert_concepts(
		finding(&result, "litellm_settings.cache", ImportStatus::Manual),
		&["response", "cach", "promptcaching"],
	);
	assert_concepts(
		finding(
			&result,
			"litellm_settings.cache_params.type",
			ImportStatus::Manual,
		),
		&["response", "cach", "promptcaching"],
	);
	finding(
		&result,
		"litellm_settings.cache_params.future_cache_option.nested",
		ImportStatus::Unsupported,
	);
}

#[test]
fn response_cache_migration_does_not_copy_values_or_change_model_routing() {
	let result = import_config("litellm", INPUT).expect("valid source configuration");
	assert_eq!(
		result.config,
		import_config("litellm", MODEL).unwrap().config
	);
	let output = serde_json::to_string(&result).unwrap();
	for value in [
		"REPRO_SYNTHETIC_PASSWORD_NOT_A_SECRET",
		"REPRO_SYNTHETIC_UNKNOWN_VALUE",
		"os.environ/REPRO_REDIS_HOST",
		"migration-repro",
	] {
		assert!(!output.contains(value), "unmapped cache value was copied");
	}
}

#[test]
fn response_cache_flags_do_not_claim_an_automatic_mapping() {
	for section in [
		"litellm_settings:\n  cache",
		"router_settings:\n  cache_responses",
	] {
		let path = section.replace(":\n  ", ".");
		for value in ["true", "false", "null", "\"REPRO_NONBOOLEAN_FLAG\""] {
			let result = import_settings(&format!("{section}: {value}\n"));
			assert_concepts(
				finding(&result, &path, ImportStatus::Manual),
				&["response", "cach"],
			);
			assert!(
				!serde_json::to_string(&result)
					.unwrap()
					.contains("REPRO_NONBOOLEAN_FLAG")
			);
		}
	}
	let absent = import_settings("");
	assert!(!absent.findings.iter().any(|finding| {
		matches!(
			finding.source_path.as_str(),
			"litellm_settings.cache" | "router_settings.cache_responses"
		)
	}));
}

#[test]
fn cache_params_without_a_flag_are_reported_without_enabling_a_cache() {
	for backend in ["redis", "local", "s3"] {
		let result = import_settings(&format!(
			"litellm_settings:\n  cache_params:\n    type: {backend}\n    ttl: 60\n    mode: default_off\n    default_in_memory_ttl: 30\n    default_in_redis_ttl: 120\n    supported_call_types: [acompletion]\n"
		));
		let backend_finding = finding(
			&result,
			"litellm_settings.cache_params.type",
			ImportStatus::Manual,
		);
		assert_concepts(backend_finding, &["backend", "response", "cach"]);
		if backend != "redis" {
			assert!(!backend_finding.message.to_lowercase().contains("redis"));
		}
		for key in [
			"ttl",
			"mode",
			"default_in_memory_ttl",
			"default_in_redis_ttl",
			"supported_call_types",
			"supported_call_types[0]",
		] {
			finding(
				&result,
				&format!("litellm_settings.cache_params.{key}"),
				ImportStatus::Manual,
			);
		}
		assert!(
			!result
				.findings
				.iter()
				.any(|finding| finding.source_path == "litellm_settings.cache")
		);
	}
}

#[test]
fn empty_or_malformed_cache_params_are_not_silently_dropped() {
	for value in ["{}", "null", "[]", "42", "\"REPRO_INVALID_CACHE_PARAMS\""] {
		let result = import_settings(&format!("litellm_settings:\n  cache_params: {value}\n"));
		finding(
			&result,
			"litellm_settings.cache_params",
			ImportStatus::Manual,
		);
		assert!(
			!serde_json::to_string(&result)
				.unwrap()
				.contains("REPRO_INVALID_CACHE_PARAMS")
		);
	}
}

#[test]
fn unknown_cache_params_preserve_paths_through_objects_arrays_and_empty_values() {
	let result = import_settings(
		r#"litellm_settings:
  cache_params:
    future_options:
      endpoints:
        - password: REPRO_NESTED_PASSWORD
        - REPRO_NESTED_ENDPOINT
      empty_object: {}
      empty_array: []
      empty_value: null
"#,
	);
	for path in [
		"litellm_settings.cache_params.future_options.endpoints[0].password",
		"litellm_settings.cache_params.future_options.endpoints[1]",
		"litellm_settings.cache_params.future_options.empty_object",
		"litellm_settings.cache_params.future_options.empty_array",
		"litellm_settings.cache_params.future_options.empty_value",
	] {
		finding(&result, path, ImportStatus::Unsupported);
	}
	let output = serde_json::to_string(&result).unwrap();
	assert!(!output.contains("REPRO_NESTED_PASSWORD"));
	assert!(!output.contains("REPRO_NESTED_ENDPOINT"));
}

#[test]
fn router_redis_does_not_imply_response_caching() {
	let result = import_settings(
		r#"router_settings:
  redis_host: os.environ/REPRO_ROUTER_REDIS_HOST
  redis_port: 6379
  redis_password: REPRO_ROUTER_PASSWORD
  redis_url: redis://REPRO_ROUTER_USER:REPRO_URL_PASSWORD@localhost:6379
  redis_db: 2
  cache_kwargs:
    ssl: true
    ssl_ca_certs: REPRO_ROUTER_TLS_PATH
"#,
	);
	for key in [
		"redis_host",
		"redis_port",
		"redis_password",
		"redis_url",
		"redis_db",
		"cache_kwargs",
		"cache_kwargs.ssl",
		"cache_kwargs.ssl_ca_certs",
	] {
		assert_concepts(
			finding(
				&result,
				&format!("router_settings.{key}"),
				ImportStatus::Manual,
			),
			&["coordination"],
		);
	}
	assert!(!result.findings.iter().any(|finding| {
		finding.source_path.starts_with("litellm_settings.cache")
			|| finding.source_path == "router_settings.cache_responses"
	}));
	let output = serde_json::to_string(&result).unwrap();
	for value in [
		"REPRO_ROUTER_REDIS_HOST",
		"REPRO_ROUTER_PASSWORD",
		"REPRO_ROUTER_USER",
		"REPRO_URL_PASSWORD",
		"REPRO_ROUTER_TLS_PATH",
	] {
		assert!(!output.contains(value), "router Redis value was copied");
	}
}

#[test]
fn mixed_router_and_response_cache_settings_keep_distinct_diagnostics() {
	let result = import_settings(
		r#"router_settings:
  redis_host: localhost
  cache_responses: true
litellm_settings:
  cache: false
  cache_params:
    type: redis
"#,
	);
	assert_concepts(
		finding(&result, "router_settings.redis_host", ImportStatus::Manual),
		&["coordination"],
	);
	for path in [
		"router_settings.cache_responses",
		"litellm_settings.cache",
		"litellm_settings.cache_params.type",
	] {
		assert_concepts(
			finding(&result, path, ImportStatus::Manual),
			&["response", "cach"],
		);
	}
	assert_concepts(
		finding(
			&result,
			"router_settings.cache_responses",
			ImportStatus::Manual,
		),
		&["not preserved"],
	);
	let false_flag = finding(&result, "litellm_settings.cache", ImportStatus::Manual);
	assert_concepts(false_flag, &["false"]);
	assert!(
		!false_flag.message.to_lowercase().contains("disabled"),
		"a false flag does not prove that all LiteLLM response caching is disabled"
	);
}
