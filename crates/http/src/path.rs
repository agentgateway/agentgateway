pub fn contains_dot_segment(path: &str) -> bool {
	let path_only = path.split_once('?').map_or(path, |(path, _)| path);
	path_only
		.split(['/', '\\'])
		.any(|segment| matches!(segment, "." | ".."))
}

pub fn is_safe_interpolated_segment(value: &str) -> bool {
	!value.is_empty() && !matches!(value, "." | "..") && !value.contains(['/', '\\'])
}

pub fn is_safe_segment(segment: &str) -> bool {
	is_safe_interpolated_segment(segment)
		&& !segment.contains(['%', '?', '#', '<', '>', '"', '`', '{', '}', '|', '^'])
		&& !segment.chars().any(|c| c.is_control() || c.is_whitespace())
}

pub fn is_safe_resource_name(name: &str) -> bool {
	!name.is_empty() && name.split('/').all(is_safe_segment)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn safe_resource_names_are_accepted() {
		for name in [
			"gemini-2.5-flash",
			"gemini@001",
			"claude-3-5-sonnet-20241022-v2:0",
			"models/gemini-2.5-flash",
			"tunedModels/abc",
			"publishers/google/models/gemini-2.5-flash",
			"arn:aws:bedrock:us-east-1:1234:application-inference-profile/my-profile",
		] {
			assert!(is_safe_resource_name(name), "{name}");
		}
	}

	#[test]
	fn unsafe_resource_names_are_rejected() {
		for name in [
			"",
			" ",
			"..",
			".",
			"gemini-2.5-flash/../../locations/global/endpoints/openapi/chat/completions",
			"gemini-2.5-flash/..",
			"/gemini-2.5-flash",
			"gemini-2.5-flash/",
			"gemini//flash",
			"gemini-2.5-flash%2F..",
			"gemini\\..\\..",
			"gemini 2.5 flash",
			"gemini\n",
			"gemini-2.5-flash?alt=sse",
			"gemini-2.5-flash#frag",
			"gemini-2.5-flash<x",
		] {
			assert!(!is_safe_resource_name(name), "{name}");
		}
	}

	#[test]
	fn interpolated_segments_allow_characters_the_caller_can_encode() {
		for value in ["user name", "user?admin=true", "user#section"] {
			assert!(is_safe_interpolated_segment(value), "{value}");
			assert!(!is_safe_segment(value), "{value}");
		}
	}

	#[test]
	fn dot_segments_are_detected_before_the_query() {
		for path in [
			"/model/../converse",
			"/models/./predict",
			"/model/foo\\..\\bar/converse",
		] {
			assert!(contains_dot_segment(path), "{path}");
		}
		assert!(!contains_dot_segment("/models/model?redirect=../other"));
	}
}
