use std::sync::Arc;

use cel::Context;
use cel::extractors::{Arguments, This};
use cel::parser::Expression;
use cel::{ExecutionError, FunctionContext, ResolveResult, Value};

pub fn insert_all(ctx: &mut Context<'_>) {
	ctx.add_function("format", format);
	ctx.add_function("parse", parse);
}

/// Represents a segment in a format string
#[derive(Debug, Clone)]
enum Segment {
	/// A literal string segment
	Literal(String),
	/// A placeholder {} that needs to be filled
	Placeholder,
}

/// A parsed format string with precomputed metadata
#[derive(Debug)]
struct FormatString {
	segments: Box<[Segment]>,
	placeholder_count: usize,
	min_capacity: usize,
}

impl FormatString {
	/// Parse a format string into segments
	/// Supports:
	/// - {} for placeholders
	/// - {{ for escaped {
	/// - }} for escaped }
	fn parse(format_str: &str) -> Result<Self, String> {
		let mut segments = Vec::new();
		let mut current_literal = String::new();
		let mut chars = format_str.chars().peekable();
		let mut placeholder_count = 0;
		let min_capacity = format_str.len();

		while let Some(ch) = chars.next() {
			match ch {
				'{' => {
					match chars.peek() {
						Some(&'{') => {
							// Escaped {{ -> single {
							chars.next();
							current_literal.push('{');
						},
						Some(&'}') => {
							// {} placeholder
							chars.next();
							if !current_literal.is_empty() {
								segments.push(Segment::Literal(current_literal.clone()));
								current_literal.clear();
							}
							segments.push(Segment::Placeholder);
							placeholder_count += 1;
						},
						Some(_) => {
							return Err("Invalid format string: '{' must be followed by '{' or '}'".to_string());
						},
						None => {
							return Err("Invalid format string: unclosed '{'".to_string());
						},
					}
				},
				'}' => {
					match chars.peek() {
						Some(&'}') => {
							// Escaped }} -> single }
							chars.next();
							current_literal.push('}');
						},
						_ => {
							return Err("Invalid format string: '}' must be escaped as '}}'".to_string());
						},
					}
				},
				_ => {
					current_literal.push(ch);
				},
			}
		}

		// Add any remaining literal
		if !current_literal.is_empty() {
			segments.push(Segment::Literal(current_literal));
		}

		Ok(FormatString {
			segments: segments.into_boxed_slice(),
			placeholder_count,
			min_capacity,
		})
	}

	/// Format the string by substituting placeholders with the given arguments
	fn format_with_args(&self, args: &[Value]) -> Result<String, String> {
		if args.len() != self.placeholder_count {
			return Err(format!(
				"Expected {} arguments, got {}",
				self.placeholder_count,
				args.len()
			));
		}

		// Pre-allocate, assuming a (probably too low) 3 char placeholder average.
		let mut result = String::with_capacity(self.min_capacity + 3 * self.placeholder_count);
		let mut arg_index = 0;

		for segment in &self.segments {
			match segment {
				Segment::Literal(s) => result.push_str(s),
				Segment::Placeholder => {
					// TODO: Implement proper Value to string conversion
					// For now, use json representation as a fallback
					let arg = &args[arg_index];
					let Some(s) = Self::value_as_string(arg) else {
						return Err(format!("Cannot convert argument {} to string", arg_index));
					};
					result.push_str(&s);
					arg_index += 1;
				},
			}
		}

		Ok(result)
	}

	/// Parse an input string according to this format pattern
	/// Returns the captured values in order
	fn parse_input(&self, input: &str) -> Result<Vec<String>, String> {
		let mut captures = Vec::with_capacity(self.placeholder_count);
		let mut input_pos = 0;
		let input_bytes = input.as_bytes();

		for (seg_idx, segment) in self.segments.iter().enumerate() {
			match segment {
				Segment::Literal(literal) => {
					// Match the literal exactly
					let literal_bytes = literal.as_bytes();
					if input_pos + literal_bytes.len() > input_bytes.len() {
						return Err(format!(
							"Input does not match format: expected '{}' at position {}",
							literal, input_pos
						));
					}
					if &input_bytes[input_pos..input_pos + literal_bytes.len()] != literal_bytes {
						return Err(format!(
							"Input does not match format: expected '{}' at position {}",
							literal, input_pos
						));
					}
					input_pos += literal_bytes.len();
				},
				Segment::Placeholder => {
					// Find what comes next in the pattern
					let next_literal = self.segments[seg_idx + 1..].iter().find_map(|s| {
						if let Segment::Literal(lit) = s {
							Some(lit.as_str())
						} else {
							None
						}
					});

					let end_pos = if let Some(next_lit) = next_literal {
						// Search for the next literal starting from current position
						let literal_bytes = next_lit.as_bytes();

						// Check if there's another placeholder before this literal
						// If so, we need to ensure we don't match ambiguously
						let has_placeholder_before_next_literal = self.segments[seg_idx + 1..]
							.iter()
							.take_while(|s| !matches!(s, Segment::Literal(l) if l == next_lit))
							.any(|s| matches!(s, Segment::Placeholder));

						if has_placeholder_before_next_literal {
							return Err(format!(
								"Ambiguous pattern: cannot determine where placeholder ends without a literal separator"
							));
						}

						// Find the first occurrence of the literal
						let first_match = input_bytes[input_pos..]
							.windows(literal_bytes.len())
							.position(|window| window == literal_bytes)
							.ok_or_else(|| {
								format!(
									"Input does not match format: could not find '{}' after position {}",
									next_lit, input_pos
								)
							})?;

						// Check if there are multiple occurrences of this literal in the remaining input
						// If so, the pattern is ambiguous
						let remaining = &input_bytes[input_pos..];
						let occurrences = remaining
							.windows(literal_bytes.len())
							.filter(|window| *window == literal_bytes)
							.count();

						if occurrences > 1 {
							return Err(format!(
								"Ambiguous pattern: literal '{}' appears {} times in remaining input, cannot determine which one to use",
								next_lit, occurrences
							));
						}

						input_pos + first_match
					} else {
						// This is the last segment, capture to the end
						input_bytes.len()
					};

					if end_pos < input_pos {
						return Err(
							"Input does not match format: placeholder cannot be empty or negative".to_string(),
						);
					}

					// Capture the substring
					let captured = std::str::from_utf8(&input_bytes[input_pos..end_pos])
						.map_err(|_| "Invalid UTF-8 in input".to_string())?;
					captures.push(captured.to_string());
					input_pos = end_pos;
				},
			}
		}

		// Check if we consumed the entire input
		if input_pos != input_bytes.len() {
			return Err(format!(
				"Input does not match format: extra characters at position {}",
				input_pos
			));
		}

		Ok(captures)
	}

	fn value_as_string(v: &Value) -> Option<String> {
		match v {
			Value::String(v) => Some(v.to_string()),
			Value::Bool(v) => Some(v.to_string()),
			Value::Int(v) => Some(v.to_string()),
			Value::UInt(v) => Some(v.to_string()),
			Value::Bytes(v) => {
				use base64::Engine;
				Some(base64::prelude::BASE64_STANDARD.encode(v.as_ref()))
			},
			_ => None,
		}
	}

}

/// format() function: "foo/{}/bar/{}".format(arg1, arg2)
pub fn format(
	_ftx: &FunctionContext,
	This(this): This<Arc<String>>,
	Arguments(args): Arguments,
) -> ResolveResult {
	// Parse the format string
	let format_string = FormatString::parse(&this).map_err(|e| ExecutionError::FunctionError {
		function: "String.format".to_owned(),
		message: e,
	})?;

	// Format with the provided arguments
	let result =
		format_string
			.format_with_args(&args)
			.map_err(|e| ExecutionError::FunctionError {
				function: "String.format".to_owned(),
				message: e,
			})?;

	Ok(Value::String(Arc::new(result)))
}

/// parse() function: "foo/a/bar/b".parse("foo/{}/bar/{}", expr)
pub fn parse(
	ftx: &FunctionContext,
	This(this): This<Arc<String>>,
	pattern: Arc<String>,
	expr: Expression,
) -> ResolveResult {
	// Parse the format/pattern string
	let format_string = FormatString::parse(&pattern).map_err(|e| ExecutionError::FunctionError {
		function: "String.parse".to_owned(),
		message: e,
	})?;

	// Parse the input string according to the pattern
	let captures = format_string
		.parse_input(&this)
		.map_err(|e| ExecutionError::FunctionError {
			function: "String.parse".to_owned(),
			message: e,
		})?;

	// Create a new scope with the captured variables
	let mut ptx = ftx.ptx.new_inner_scope();
	for (i, capture) in captures.iter().enumerate() {
		// Variable names: _1, _2, _3, etc.
		let var_name = format!("_{}", i + 1);
		ptx.add_variable_from_value(&var_name, Value::String(Arc::new(capture.clone())));
	}

	// Evaluate the expression in the new scope
	ptx.resolve(&expr)
}
