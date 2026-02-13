use bytes::Bytes;
use http::Method;
use serde_json::json;

use super::*;
use crate::http::Body;

/// Helper to build a test request with various fields populated
fn build_test_request() -> crate::http::Request {
	let mut req = ::http::Request::builder()
		.method(Method::POST)
		.uri("http://example.com/api/test")
		.header("x-custom-header", "test-value")
		.header("content-type", "application/json")
		.body(Body::from(r#"{"key": "value"}"#))
		.unwrap();

	// Add JWT claims
	let claims = jwt::Claims {
		inner: serde_json::Map::from_iter(vec![
			("sub".to_string(), json!("user123")),
			("iss".to_string(), json!("agentgateway.dev")),
			("exp".to_string(), json!(1900650294)),
		]),
		jwt: secrecy::SecretString::new("fake.jwt.token".into()),
	};
	req.extensions_mut().insert(claims);

	// Add source context
	let source = SourceContext {
		address: "127.0.0.1".parse().unwrap(),
		port: 54321,
		tls: None,
	};
	req.extensions_mut().insert(source);

	// Add backend context
	let backend = BackendContext {
		name: "test-backend".into(),
		backend_type: BackendType::Service,
		protocol: BackendProtocol::http,
	};
	req.extensions_mut().insert(backend);
	req
		.extensions_mut()
		.insert(RequestStartTime("now".to_string()));

	// Add LLM context
	let llm = LLMContext {
		streaming: false,
		request_model: "gpt-4".into(),
		response_model: Some("gpt-4-turbo".into()),
		provider: "openai".into(),
		input_tokens: Some(100),
		output_tokens: Some(50),
		total_tokens: Some(150),
		first_token: None,
		count_tokens: None,
		reasoning_tokens: None,
		cache_creation_input_tokens: None,
		cached_input_tokens: None,
		prompt: None,
		completion: Some(vec!["Hello world".to_string()]),
		params: llm::LLMRequestParams::default(),
	};
	req.extensions_mut().insert(llm);

	req
}

#[test]
fn test_snapshot_matches_ref() {
	let mut req = build_test_request();
	let snapshot = snapshot_request(&mut req);
	let req = build_test_request();
	let snapshot_exec =
		Executor::new_logger(Some(&snapshot), None, snapshot.llm.as_ref(), None, None);
	let ref_executor = Executor::new_request(&req);

	assert_eq!(exec_to_json(&ref_executor), exec_to_json(&snapshot_exec));
}

#[test]
fn test_executor_snapshot_round_trip() {
	let mut req = build_test_request();
	let req_snapshot = snapshot_request(&mut req);

	// Create executor from snapshot
	let executor1 = Executor::new_logger(Some(&req_snapshot), None, None, None, None);

	// Serialize to JSON
	let json = exec_to_json(&executor1);

	// Deserialize into ExecutorSerde
	let exec_snapshot: ExecutorSerde =
		serde_json::from_value(json.clone()).expect("failed to deserialize ExecutorSerde");

	// Build executor from ExecutorSerde
	let executor2 = exec_snapshot.as_executor();

	// Serialize again
	let json2 = exec_to_json(&executor2);

	// They should be identical
	assert_eq!(json, json2, "Round-trip serialization mismatch");
}

#[test]
fn test_executor_round_trip() {
	let exec = full_example_executor();
	let executor1 = exec.as_executor();

	// Serialize to JSON
	let json = exec_to_json(&executor1);

	// Deserialize into ExecutorSerde
	let exec_snapshot: ExecutorSerde =
		serde_json::from_value(json.clone()).expect("failed to deserialize ExecutorSerde");

	// Build executor from ExecutorSerde
	let executor2 = exec_snapshot.as_executor();

	// Serialize again
	let json2 = exec_to_json(&executor2);

	// They should be identical
	assert_eq!(json, json2, "Round-trip serialization mismatch");
}

#[test]
fn test_executor_serde_complete() {
	let exec = full_example_executor();
	let json1 = serde_json::to_value(&exec).expect("failed to serialize executor2");

	// Build executor from ExecutorSerde
	let executor2 = exec.as_executor();

	let json3 = exec_to_json(&executor2);
	assert_eq!(json1, json3, "Round-trip serialization mismatch");
}

fn exec_to_json(exec: &Executor) -> serde_json::Value {
	let expr = Expression::new_strict("variables()").expect("failed to compile");
	let cel_value = exec.eval(&expr).expect("failed to evaluate");
	cel_value.json().expect("failed to convert to JSON")
}

#[test]
fn test_executor_snapshot_json_to_cel() {
	// Create a JSON representation manually
	let json = json!({
		"request": {
			"method": "GET",
			"uri": "http://example.com/test",
			"path": "/test",
			"host": "example.com",
			"scheme": "http",
			"version": "HTTP/1.1",
			"headers": {
				"x-test": "value"
			}
		},
		"source": {
			"address": "10.0.0.1",
			"port": 12345
		},
		"backend": {
			"name": "my-backend",
			"type": "service",
			"protocol": "http"
		},
		"jwt": {
			"sub": "test-user",
			"role": "admin"
		}
	});

	// Deserialize into ExecutorSerde
	let snapshot: ExecutorSerde =
		serde_json::from_value(json.clone()).expect("failed to deserialize");

	// Build executor
	let executor = snapshot.as_executor();

	// Evaluate variables()
	let expr = Expression::new_strict("variables()").expect("failed to compile");
	let cel_value = executor.eval(&expr).expect("failed to evaluate");
	let cel_json = cel_value.json().expect("failed to convert to JSON");

	// Verify key fields match
	assert_eq!(cel_json["request"]["method"], "GET");
	assert_eq!(cel_json["request"]["path"], "/test");
	assert_eq!(cel_json["source"]["address"], "10.0.0.1");
	assert_eq!(cel_json["backend"]["name"], "my-backend");
	assert_eq!(cel_json["jwt"]["sub"], "test-user");
}

#[test]
fn test_executor_minimal_json() {
	// Create a JSON representation manually
	let json = json!({
		"request": {
		},
		"response": {
		},
		"source": {
		},
		"backend": {
		},
		"jwt": {
		}
	});

	// Deserialize into ExecutorSerde
	let _: ExecutorSerde = serde_json::from_value(json.clone()).expect("failed to deserialize");
}
#[test]
fn test_buffered_body_serialization() {
	let body_data = b"Hello, World!";
	let buffered_body = BufferedBody(Bytes::from_static(body_data));

	// Serialize
	let json = serde_json::to_value(&buffered_body).expect("failed to serialize");

	// Should be base64 encoded
	assert!(json.is_string());
	let _encoded = json.as_str().unwrap();

	// Deserialize
	let deserialized: BufferedBody = serde_json::from_value(json).expect("failed to deserialize");

	// Should match original
	assert_eq!(buffered_body.0, deserialized.0);
}

#[test]
fn test_extension_or_direct_serialization() {
	// Test Direct with Some
	let value = SourceContext {
		address: "192.168.1.1".parse().unwrap(),
		port: 8080,
		tls: None,
	};
	let ext_or_direct: ExtensionOrDirect<SourceContext> = ExtensionOrDirect::Direct(Some(&value));
	let json = serde_json::to_value(&ext_or_direct).expect("failed to serialize");
	assert_eq!(json["address"], "192.168.1.1");
	assert_eq!(json["port"], 8080);

	// Test Direct with None
	let ext_or_direct_none: ExtensionOrDirect<SourceContext> = ExtensionOrDirect::Direct(None);
	let json_none = serde_json::to_value(&ext_or_direct_none).expect("failed to serialize");
	assert!(json_none.is_null());
}
