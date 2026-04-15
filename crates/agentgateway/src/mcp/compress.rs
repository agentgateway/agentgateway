#[cfg(feature = "schema")]
use schemars::JsonSchema;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum CompressionFormat {
	Markdown,
	Tsv,
	Csv,
	None,
}

impl std::fmt::Display for CompressionFormat {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			CompressionFormat::Markdown => write!(f, "markdown"),
			CompressionFormat::Tsv => write!(f, "tsv"),
			CompressionFormat::Csv => write!(f, "csv"),
			CompressionFormat::None => write!(f, "none"),
		}
	}
}

/// Entry point for response compression
pub fn compress_response(content: &str, format: CompressionFormat) -> Option<String> {
	if format == CompressionFormat::None {
		return None;
	}

	let json: Value = serde_json::from_str(content).ok()?;

	match json {
		// Direct array of objects
		Value::Array(arr) => {
			if arr.iter().all(|v| v.is_object()) && !arr.is_empty() {
				Some(convert_array_to_table(&arr, format))
			} else {
				None
			}
		},
		// Object with array field
		Value::Object(ref obj) => {
			// Look for a top-level key whose value is an array of objects
			for (key, value) in obj.iter() {
				if let Value::Array(arr) = value
					&& arr.iter().all(|v| v.is_object())
					&& !arr.is_empty()
				{
					let mut result = String::new();

					// Add scalar fields as header lines
					for (k, v) in obj.iter() {
						if k != key && !v.is_array() {
							result.push_str(&format!("{}: {}\n", k, render_value(v)));
						}
					}
					if !result.is_empty() {
						result.push('\n');
					}

					result.push_str(&convert_array_to_table(arr, format));
					return Some(result);
				}
			}
			None
		},
		_ => None,
	}
}

fn convert_array_to_table(arr: &[Value], format: CompressionFormat) -> String {
	if arr.is_empty() {
		return String::new();
	}

	// Collect all unique keys from all objects
	let mut all_keys = Vec::new();
	let mut key_set = std::collections::HashSet::new();

	// Use keys from first object to maintain order
	if let Some(Value::Object(first_obj)) = arr.first() {
		for key in first_obj.keys() {
			all_keys.push(key.clone());
			key_set.insert(key.clone());
		}
	}

	// Add any additional keys from other objects
	for item in arr.iter() {
		if let Value::Object(obj) = item {
			for key in obj.keys() {
				if !key_set.contains(key) {
					all_keys.push(key.clone());
					key_set.insert(key.clone());
				}
			}
		}
	}

	match format {
		CompressionFormat::Markdown => {
			let mut result = String::new();

			// Header row
			result.push_str("| ");
			result.push_str(
				&all_keys
					.iter()
					.map(|k| escape_markdown(k))
					.collect::<Vec<_>>()
					.join(" | "),
			);
			result.push_str(" |\n");

			// Separator row
			result.push('|');
			for _ in &all_keys {
				result.push_str("------|");
			}
			result.push('\n');

			// Data rows
			for item in arr {
				if let Value::Object(obj) = item {
					result.push_str("| ");
					let values: Vec<String> = all_keys
						.iter()
						.map(|key| escape_markdown(&obj.get(key).map(render_value).unwrap_or_default()))
						.collect();
					result.push_str(&values.join(" | "));
					result.push_str(" |\n");
				}
			}

			result
		},
		CompressionFormat::Tsv => {
			let mut result = String::new();

			// Header row
			result.push_str(
				&all_keys
					.iter()
					.map(|k| escape_tsv(k))
					.collect::<Vec<_>>()
					.join("\t"),
			);
			result.push('\n');

			// Data rows
			for item in arr {
				if let Value::Object(obj) = item {
					let values: Vec<String> = all_keys
						.iter()
						.map(|key| escape_tsv(&obj.get(key).map(render_value).unwrap_or_default()))
						.collect();
					result.push_str(&values.join("\t"));
					result.push('\n');
				}
			}

			result
		},
		CompressionFormat::Csv => {
			let mut result = String::new();

			// Header row
			result.push_str(&all_keys.join(","));
			result.push('\n');

			// Data rows
			for item in arr {
				if let Value::Object(obj) = item {
					let values: Vec<String> = all_keys
						.iter()
						.map(|key| {
							obj
								.get(key)
								.map(|v| escape_csv_value(&render_value(v)))
								.unwrap_or_default()
						})
						.collect();
					result.push_str(&values.join(","));
					result.push('\n');
				}
			}

			result
		},
		CompressionFormat::None => String::new(),
	}
}

fn render_value(value: &Value) -> String {
	match value {
		Value::String(s) => s.clone(),
		Value::Number(n) => n.to_string(),
		Value::Bool(b) => {
			if *b {
				"true".to_string()
			} else {
				"false".to_string()
			}
		},
		Value::Null => String::new(),
		Value::Array(arr) => {
			if arr.len() <= 5 {
				// Render flat arrays as comma-separated if all elements are primitives
				if arr.iter().all(|v| !v.is_object() && !v.is_array()) {
					arr.iter().map(render_value).collect::<Vec<_>>().join(",")
				} else if arr.iter().all(|v| v.is_object()) {
					format!("[{{...}} x {}]", arr.len())
				} else {
					"{...}".to_string()
				}
			} else {
				// For arrays > 5 items, show first 5 + count
				if arr.iter().all(|v| !v.is_object() && !v.is_array()) {
					let first_five: Vec<String> = arr.iter().take(5).map(render_value).collect();
					format!("{} (+{} more)", first_five.join(","), arr.len() - 5)
				} else if arr.iter().all(|v| v.is_object()) {
					format!("[{{...}} x {}]", arr.len())
				} else {
					"{...}".to_string()
				}
			}
		},
		Value::Object(_) => "{...}".to_string(),
	}
}

fn escape_markdown(value: &str) -> String {
	value.replace('|', "\\|").replace('\n', " ")
}

fn escape_tsv(value: &str) -> String {
	value.replace('\t', " ").replace('\n', " ")
}

fn escape_csv_value(value: &str) -> String {
	if value.contains(',') || value.contains('"') || value.contains('\n') {
		format!("\"{}\"", value.replace('"', "\"\""))
	} else {
		value.to_string()
	}
}

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
	fn test_markdown_escapes_pipe() {
		let input = r#"[{"name": "a|b", "value": "ok"}]"#;
		let result = compress_response(input, CompressionFormat::Markdown).unwrap();
		assert!(result.contains("a\\|b"));
		assert!(!result.contains("| a|b |"));
	}

	#[test]
	fn test_tsv_escapes_tab_and_newline() {
		let input = r#"[{"name": "a\tb", "desc": "line1\nline2"}]"#;
		let result = compress_response(input, CompressionFormat::Tsv).unwrap();
		// Tabs and newlines in values should be replaced with spaces
		assert!(!result.contains("a\tb"));
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
