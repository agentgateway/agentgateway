use std::path::Path;

fn normalize_mcp_snapshot(value: &mut serde_json::Value) {
	match value {
		serde_json::Value::Array(values) => {
			for value in values {
				normalize_mcp_snapshot(value);
			}
		},
		serde_json::Value::Object(map) => {
			for (key, value) in map.iter_mut() {
				if matches!(
					key.as_str(),
					"session_id" | "event_id" | "last_event_id" | "replayed_event_id"
				) && value.is_string()
				{
					*value = serde_json::Value::String(format!("[{key}]"));
					continue;
				}
				normalize_mcp_snapshot(value);
			}
		},
		_ => {},
	}
}

fn mcp_snapshot_path_and_name(relative_path: &str) -> (String, String) {
	let rel = Path::new(relative_path);
	let parent = rel.parent().unwrap_or_else(|| Path::new(""));
	let stem = rel
		.file_stem()
		.unwrap_or_else(|| panic!("{relative_path}: missing filename"))
		.to_string_lossy();
	(
		if parent.as_os_str().is_empty() {
			".".to_string()
		} else {
			parent.display().to_string()
		},
		stem.to_string(),
	)
}

pub(super) fn assert_mcp_json_snapshot(relative_path: &str, mut report: serde_json::Value) {
	normalize_mcp_snapshot(&mut report);
	let (snapshot_path, snapshot_name) = mcp_snapshot_path_and_name(relative_path);
	insta::with_settings!({
		prepend_module_to_snapshot => false,
		snapshot_path => snapshot_path,
	}, {
		insta::assert_json_snapshot!(snapshot_name, report);
	});
}
