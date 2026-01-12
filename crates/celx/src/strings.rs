// Copied from https://raw.githubusercontent.com/Kuadrant/wasm-shim/refs/heads/main/src/data/cel/strings.rs
// under Apache 2.0 license (https://github.com/Kuadrant/wasm-shim/blob/main/LICENSE)
// TODO: https://github.com/cel-rust/cel-rust/issues/103, have this upstreamed

use std::sync::Arc;

use ::cel::extractors::{Arguments, This};
use ::cel::{ExecutionError, ResolveResult, Value};
use cel::Context;

pub fn insert_all(ctx: &mut Context<'_>) {
	ctx.add_function("charAt", char_at);
	ctx.add_function("indexOf", index_of);
	ctx.add_function("join", join);
	ctx.add_function("lastIndexOf", last_index_of);
	ctx.add_function("lowerAscii", lower_ascii);
	ctx.add_function("upperAscii", upper_ascii);
	ctx.add_function("trim", trim);
	ctx.add_function("replace", replace);
	ctx.add_function("split", split);
	ctx.add_function("substring", substring);
}

pub fn char_at(This(this): This<Arc<String>>, arg: i64) -> ResolveResult {
	match this.chars().nth(arg as usize) {
		None => Err(ExecutionError::FunctionError {
			function: "String.charAt".to_owned(),
			message: format!("No index {arg} on `{this}`"),
		}),
		Some(c) => Ok(c.to_string().into()),
	}
}

pub fn index_of(
	This(this): This<Arc<String>>,
	arg: Arc<String>,
	Arguments(args): Arguments,
) -> ResolveResult {
	match args.len() {
		1 => match this.find(&*arg) {
			None => Ok((-1).into()),
			Some(idx) => Ok((idx as u64).into()),
		},
		2 => {
			let base = match args[1] {
				Value::Int(i) => i as usize,
				Value::UInt(u) => u as usize,
				_ => {
					return Err(ExecutionError::FunctionError {
						function: "String.indexOf".to_owned(),
						message: format!("Expects 2nd argument to be an Integer, got `{:?}`", args[1]),
					});
				},
			};
			if base >= this.len() {
				return Ok((-1).into());
			}
			match this[base..].find(&*arg) {
				None => Ok((-1).into()),
				Some(idx) => Ok(Value::UInt((base + idx) as u64)),
			}
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.indexOf".to_owned(),
			message: format!("Expects 2 arguments at most, got `{args:?}`!"),
		}),
	}
}

pub fn last_index_of(
	This(this): This<Arc<String>>,
	arg: Arc<String>,
	Arguments(args): Arguments,
) -> ResolveResult {
	match args.len() {
		1 => match this.rfind(&*arg) {
			None => Ok((-1).into()),
			Some(idx) => Ok((idx as u64).into()),
		},
		2 => {
			let base = match args[1] {
				Value::Int(i) => i as usize,
				Value::UInt(u) => u as usize,
				_ => {
					return Err(ExecutionError::FunctionError {
						function: "String.lastIndexOf".to_owned(),
						message: format!("Expects 2nd argument to be an Integer, got `{:?}`", args[1]),
					});
				},
			};
			if base >= this.len() {
				return Ok((-1).into());
			}
			match this[base..].rfind(&*arg) {
				None => Ok((-1).into()),
				Some(idx) => Ok(Value::UInt(idx as u64)),
			}
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.lastIndexOf".to_owned(),
			message: format!("Expects 2 arguments at most, got `{args:?}`!"),
		}),
	}
}

pub fn join(This(this): This<Arc<Vec<Value>>>, Arguments(args): Arguments) -> ResolveResult {
	let separator = args
		.first()
		.map(|v| match v {
			Value::String(s) => Ok(s.as_str()),
			_ => Err(ExecutionError::FunctionError {
				function: "List.join".to_owned(),
				message: format!("Expects seperator to be a String, got `{v:?}`!"),
			}),
		})
		.unwrap_or(Ok(""))?;
	Ok(
		this
			.iter()
			.map(|v| match v {
				Value::String(s) => Ok(s.as_str().to_string()),
				_ => Err(ExecutionError::FunctionError {
					function: "List.join".to_owned(),
					message: "Expects a list of String values!".to_owned(),
				}),
			})
			.collect::<Result<Vec<_>, _>>()?
			.join(separator)
			.into(),
	)
}

pub fn lower_ascii(This(this): This<Arc<String>>) -> ResolveResult {
	Ok(this.to_ascii_lowercase().into())
}

pub fn upper_ascii(This(this): This<Arc<String>>) -> ResolveResult {
	Ok(this.to_ascii_uppercase().into())
}

pub fn trim(This(this): This<Arc<String>>) -> ResolveResult {
	Ok(this.trim().into())
}

pub fn replace(This(this): This<Arc<String>>, Arguments(args): Arguments) -> ResolveResult {
	match args.len() {
		count @ 2..=3 => {
			let from = match &args[0] {
				Value::String(s) => s.as_str(),
				_ => Err(ExecutionError::FunctionError {
					function: "String.replace".to_owned(),
					message: format!(
						"First argument of type String expected, got `{:?}`",
						args[0]
					),
				})?,
			};
			let to = match &args[1] {
				Value::String(s) => s.as_str(),
				_ => Err(ExecutionError::FunctionError {
					function: "String.replace".to_owned(),
					message: format!(
						"Second argument of type String expected, got `{:?}`",
						args[1]
					),
				})?,
			};
			if count == 3 {
				let n = match &args[2] {
					Value::Int(i) => *i as usize,
					Value::UInt(u) => *u as usize,
					_ => Err(ExecutionError::FunctionError {
						function: "String.replace".to_owned(),
						message: format!(
							"Third argument of type Integer expected, got `{:?}`",
							args[2]
						),
					})?,
				};
				Ok(this.replacen(from, to, n).into())
			} else {
				Ok(this.replace(from, to).into())
			}
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.replace".to_owned(),
			message: format!("Expects 2 or 3 arguments, got {args:?}"),
		}),
	}
}

pub fn split(This(this): This<Arc<String>>, Arguments(args): Arguments) -> ResolveResult {
	match args.len() {
		count @ 1..=2 => {
			let sep = match &args[0] {
				Value::String(sep) => sep.as_str(),
				_ => {
					return Err(ExecutionError::FunctionError {
						function: "String.split".to_string(),
						message: format!(
							"Expects a first argument of type String, got `{:?}`",
							args[0]
						),
					});
				},
			};
			let list = if count == 2 {
				let pos = match &args[1] {
					Value::UInt(u) => *u as usize,
					Value::Int(i) => *i as usize,
					_ => Err(ExecutionError::FunctionError {
						function: "String.split".to_string(),
						message: format!(
							"Expects a second argument of type Integer, got `{:?}`",
							args[1]
						),
					})?,
				};
				this
					.splitn(pos, sep)
					.map(|s| Value::String(s.to_owned().into()))
					.collect::<Vec<Value>>()
			} else {
				this
					.split(sep)
					.map(|s| Value::String(s.to_owned().into()))
					.collect::<Vec<Value>>()
			};
			Ok(list.into())
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.split".to_owned(),
			message: format!("Expects at most 2 arguments, got {args:?}"),
		}),
	}
}

pub fn substring(This(this): This<Arc<String>>, Arguments(args): Arguments) -> ResolveResult {
	match args.len() {
		count @ 1..=2 => {
			let start = match &args[0] {
				Value::Int(i) => *i as usize,
				Value::UInt(u) => *u as usize,
				_ => Err(ExecutionError::FunctionError {
					function: "String.substring".to_string(),
					message: format!(
						"Expects a first argument of type Integer, got `{:?}`",
						args[0]
					),
				})?,
			};
			let end = if count == 2 {
				match &args[1] {
					Value::Int(i) => *i as usize,
					Value::UInt(u) => *u as usize,
					_ => Err(ExecutionError::FunctionError {
						function: "String.substring".to_string(),
						message: format!(
							"Expects a second argument of type Integer, got `{:?}`",
							args[0]
						),
					})?,
				}
			} else {
				this.chars().count()
			};
			if end < start {
				Err(ExecutionError::FunctionError {
					function: "String.substring".to_string(),
					message: format!("Can't have end be before the start: `{end} < {start}"),
				})?
			}
			Ok(
				this
					.chars()
					.skip(start)
					.take(end - start)
					.collect::<String>()
					.into(),
			)
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.substring".to_owned(),
			message: format!("Expects at most 2 arguments, got {args:?}"),
		}),
	}
}
