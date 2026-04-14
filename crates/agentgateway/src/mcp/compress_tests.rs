#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_json_array_to_markdown() {
		let input = r#"[
			{"name": "svc-a", "namespace": "default", "status": "Running"},
			{"name": "svc-b", "namespace": "default", "status": "Pending"}
		]"#;
		
		let result = compress_response(input, CompressionFormat::Markdown);
		assert!(result.is_some());
		
		let output = result.unwrap();
		assert!(output.contains("| name | namespace | status |"));
		assert!(output.contains("| svc-a | default | Running |"));
		assert!(output.contains("| svc-b | default | Pending |"));
	}

	#[test]
	fn test_json_array_to_tsv() {
		let input = r#"[
			{"name": "svc-a", "namespace": "default", "status": "Running"},
			{"name": "svc-b", "namespace": "default", "status": "Pending"}
		]"#;
		
		let result = compress_response(input, CompressionFormat::Tsv);
		assert!(result.is_some());
		
		let output = result.unwrap();
		assert!(output.contains("name\tnamespace\tstatus"));
		assert!(output.contains("svc-a\tdefault\tRunning"));
	}

	#[test]
	fn test_json_array_to_csv() {
		let input = r#"[
			{"name": "svc-a", "namespace": "default", "status": "Running"},
			{"name": "svc-b", "namespace": "default", "status": "Pending"}
		]"#;
		
		let result = compress_response(input, CompressionFormat::Csv);
		assert!(result.is_some());
		
		let output = result.unwrap();
		assert!(output.contains("name,namespace,status"));
		assert!(output.contains("svc-a,default,Running"));
	}

	#[test]
	fn test_wrapper_object() {
		let input = r#"{
			"items": [
				{"name": "svc-a", "status": "Running"},
				{"name": "svc-b", "status": "Pending"}
			],
			"count": 2
		}"#;
		
		let result = compress_response(input, CompressionFormat::Markdown);
		assert!(result.is_some());
		
		let output = result.unwrap();
		assert!(output.contains("count: 2"));
		assert!(output.contains("| name | status |"));
	}

	#[test]
	fn test_non_convertible_passthrough() {
		let input = r#"{"message": "hello world"}"#;
		
		let result = compress_response(input, CompressionFormat::Markdown);
		assert!(result.is_none());
	}

	#[test]
	fn test_array_value_rendering() {
		let input = r#"[
			{"name": "test", "tags": ["a", "b", "c"]},
			{"name": "test2", "tags": ["x", "y", "z", "w", "v", "u", "t"]}
		]"#;
		
		let result = compress_response(input, CompressionFormat::Markdown);
		assert!(result.is_some());
		
		let output = result.unwrap();
		assert!(output.contains("a,b,c"));
		assert!(output.contains("x,y,z,w,v (+2 more)"));
	}

	#[test]
	fn test_nested_objects() {
		let input = r#"[
			{"name": "test", "metadata": {"created": "2023-01-01"}},
			{"name": "test2", "metadata": {"created": "2023-01-02"}}
		]"#;
		
		let result = compress_response(input, CompressionFormat::Markdown);
		assert!(result.is_some());
		
		let output = result.unwrap();
		assert!(output.contains("{...}"));
	}

	#[test]
	fn test_empty_array() {
		let input = r#"[]"#;
		
		let result = compress_response(input, CompressionFormat::Markdown);
		assert!(result.is_none());
	}

	#[test]
	fn test_single_object() {
		let input = r#"[{"name": "single", "status": "ok"}]"#;
		
		let result = compress_response(input, CompressionFormat::Markdown);
		assert!(result.is_some());
		
		let output = result.unwrap();
		assert!(output.contains("| name | status |"));
		assert!(output.contains("| single | ok |"));
	}
}