use cel::Context;
use cel::context::{MapResolver, VariableResolver};
use cel::extractors::{Argument, This};
use cel::objects::StringValue;
use cel::{ExecutionError, FunctionContext, ResolveResult, Value};
use std::collections::HashSet;
use std::sync::Arc;

pub fn insert_all(ctx: &mut Context) {
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

#[derive(Debug, Clone)]
enum ParseSegment {
	Literal(String),
	Placeholder(Option<String>),
}

/// A parsed format string with precomputed metadata
#[derive(Debug)]
struct FormatString {
	segments: Box<[Segment]>,
	placeholder_count: usize,
	min_capacity: usize,
}

#[derive(Debug)]
struct ParsePattern {
	segments: Box<[ParseSegment]>,
	placeholder_count: usize,
	min_capacity: usize,
}

#[derive(Debug)]
struct ParseCapture {
	name: Option<String>,
	value: String,
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
					let Some(s) = arg.as_str().ok() else {
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
}

impl ParsePattern {
	/// Parse a format string into segments for parse() patterns.
	/// Supports:
	/// - {} for ignored placeholders
	/// - {name} for named placeholders
	/// - {{ for escaped {
	/// - }} for escaped }
	fn parse(pattern: &str) -> Result<Self, String> {
		let mut segments = Vec::new();
		let mut current_literal = String::new();
		let mut chars = pattern.chars().peekable();
		let mut placeholder_count = 0;
		let min_capacity = pattern.len();
		let mut capture_names = HashSet::new();

		while let Some(ch) = chars.next() {
			match ch {
				'{' => {
					match chars.peek() {
						Some(&'{') => {
							chars.next();
							current_literal.push('{');
						},
						Some(&'}') => {
							chars.next();
							if !current_literal.is_empty() {
								segments.push(ParseSegment::Literal(current_literal.clone()));
								current_literal.clear();
							}
							segments.push(ParseSegment::Placeholder(None));
							placeholder_count += 1;
						},
						Some(_) => {
							let mut name = String::new();
							let mut closed = false;
							while let Some(next_ch) = chars.next() {
								if next_ch == '}' {
									closed = true;
									break;
								}
								name.push(next_ch);
							}
							if !closed {
								return Err("Invalid parse pattern: unclosed '{'".to_string());
							}
							if !Self::is_simple_var_name(&name) {
								return Err(format!(
									"Invalid parse pattern: capture name '{}' must be a simple identifier",
									name
								));
							}
							if !capture_names.insert(name.clone()) {
								return Err(format!(
									"Invalid parse pattern: duplicate capture name '{}'",
									name
								));
							}
							if !current_literal.is_empty() {
								segments.push(ParseSegment::Literal(current_literal.clone()));
								current_literal.clear();
							}
							segments.push(ParseSegment::Placeholder(Some(name)));
							placeholder_count += 1;
						},
						None => {
							return Err("Invalid parse pattern: unclosed '{'".to_string());
						},
					}
				},
				'}' => {
					match chars.peek() {
						Some(&'}') => {
							chars.next();
							current_literal.push('}');
						},
						_ => {
							return Err("Invalid parse pattern: '}' must be escaped as '}}'".to_string());
						},
					}
				},
				_ => {
					current_literal.push(ch);
				},
			}
		}

		if !current_literal.is_empty() {
			segments.push(ParseSegment::Literal(current_literal));
		}

		Ok(ParsePattern {
			segments: segments.into_boxed_slice(),
			placeholder_count,
			min_capacity,
		})
	}

	fn is_simple_var_name(name: &str) -> bool {
		let mut chars = name.chars();
		let Some(first) = chars.next() else {
			return false;
		};
		if !(first.is_ascii_alphabetic() || first == '_') {
			return false;
		}
		chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
	}

	/// Parse an input string according to the format pattern
	/// Returns captured values in order with optional capture names.
	fn parse_input(&self, input: &str) -> Result<Vec<ParseCapture>, String> {
		let mut captures = Vec::with_capacity(self.placeholder_count);
		let mut input_pos = 0;
		let input_bytes = input.as_bytes();

		for (seg_idx, segment) in self.segments.iter().enumerate() {
			match segment {
				ParseSegment::Literal(literal) => {
					let literal_bytes = literal.as_bytes();
					if input_pos + literal_bytes.len() > input_bytes.len() {
						return Err(format!(
							"Input does not match parse pattern: expected '{}' at position {}",
							literal, input_pos
						));
					}
					if &input_bytes[input_pos..input_pos + literal_bytes.len()] != literal_bytes {
						return Err(format!(
							"Input does not match parse pattern: expected '{}' at position {}",
							literal, input_pos
						));
					}
					input_pos += literal_bytes.len();
				},
				ParseSegment::Placeholder(name) => {
					let next_literal = self.segments[seg_idx + 1..].iter().find_map(|s| {
						if let ParseSegment::Literal(lit) = s {
							Some(lit.as_str())
						} else {
							None
						}
					});

					let end_pos = if let Some(next_lit) = next_literal {
						let literal_bytes = next_lit.as_bytes();

						let has_placeholder_before_next_literal = self.segments[seg_idx + 1..]
							.iter()
							.take_while(|s| !matches!(s, ParseSegment::Literal(l) if l == next_lit))
							.any(|s| matches!(s, ParseSegment::Placeholder(_)));

						if has_placeholder_before_next_literal {
							return Err(format!(
								"Ambiguous pattern: cannot determine where placeholder ends without a literal separator"
							));
						}

							let first_match = input_bytes[input_pos..]
								.windows(literal_bytes.len())
								.position(|window| window == literal_bytes)
							.ok_or_else(|| {
								format!(
									"Input does not match parse pattern: could not find '{}' after position {}",
									next_lit, input_pos
								)
								})?;

							let occurrences = input_bytes[input_pos..]
								.windows(literal_bytes.len())
								.filter(|window| *window == literal_bytes)
								.count();
							let has_later_literal = self.segments[seg_idx + 1..]
								.iter()
								.skip_while(|s| !matches!(s, ParseSegment::Literal(l) if l == next_lit))
								.skip(1)
								.any(|s| matches!(s, ParseSegment::Literal(_)));
							if occurrences > 1 && !has_later_literal {
								return Err(format!(
									"Ambiguous pattern: literal '{}' appears {} times in remaining input, cannot determine which one to use",
									next_lit, occurrences
								));
							}

							input_pos + first_match
						} else {
							input_bytes.len()
						};

					if end_pos < input_pos {
						return Err(
							"Input does not match parse pattern: capture cannot be empty or negative".to_string(),
						);
					}

					let captured = std::str::from_utf8(&input_bytes[input_pos..end_pos])
						.map_err(|_| "Invalid UTF-8 in input".to_string())?
						.to_string();

					captures.push(ParseCapture {
						name: name.clone(),
						value: captured,
					});
					input_pos = end_pos;
				},
			}
		}

		if input_pos != input_bytes.len() {
			return Err(format!(
				"Input does not match parse pattern: extra characters at position {}",
				input_pos
			));
		}

		Ok(captures)
	}
}

/// format() function: "foo/{}/bar/{}".format(arg1, arg2)
pub fn format<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	// Parse the format string
	let format_string = FormatString::parse(&this).map_err(|e| ExecutionError::FunctionError {
		function: "String.format".to_owned(),
		message: e,
	})?;

	let args: Vec<_> = ftx.value_iter().collect::<Result<_, _>>()?;
	// Format with the provided arguments
	let result = format_string
		.format_with_args(args.as_slice())
		.map_err(|e| ExecutionError::FunctionError {
			function: "String.format".to_owned(),
			message: e,
		})?;

	Ok(Value::String(result.into()))
}

/// parse() function: "foo/a/bar/b".parse("foo/{prefix}/{}/{}", expr)
pub fn parse<'a, 'rf, 'b>(
	ftx: &'b mut FunctionContext<'a, 'rf>,
	this: This,
	pattern: Argument,
	expr: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let pattern: StringValue = pattern.load_value(ftx)?;
	let expr = expr.load_expression(ftx)?;

	// Parse the parse pattern string
	let pattern = ParsePattern::parse(&pattern).map_err(|e| ExecutionError::FunctionError {
		function: "String.parse".to_owned(),
		message: e,
	})?;

	// Parse the input string according to the pattern
	let captures = pattern
		.parse_input(&this)
		.map_err(|e| ExecutionError::FunctionError {
			function: "String.parse".to_owned(),
			message: e,
		})?;

	let base_vars: &'rf dyn VariableResolver<'a> = ftx.vars();
	let mut vars = MapResolver::with_base(base_vars);

	// Create a new scope with the captured variables
	for capture in &captures {
		if let Some(name) = capture.name.as_deref() {
			vars.add_variable_from_value(
				name,
				Value::String(StringValue::Owned(Arc::from(capture.value.as_str()))),
			);
		}
	}

	// Evaluate the expression in the new scope
	let v = Value::resolve(expr, ftx.ptx, &vars)?;
	Ok(v)
}
