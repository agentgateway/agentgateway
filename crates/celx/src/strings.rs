// Copied from https://raw.githubusercontent.com/Kuadrant/wasm-shim/refs/heads/main/src/data/cel/strings.rs
// under Apache 2.0 license (https://github.com/Kuadrant/wasm-shim/blob/main/LICENSE)
// TODO: https://github.com/cel-rust/cel-rust/issues/103, have this upstreamed

use cel::Context;
use cel::extractors::{Argument, This};
use cel::objects::StringValue;
use cel::{ExecutionError, FunctionContext, ResolveResult, Value};

pub fn insert_all(ctx: &mut Context) {
	ctx.add_function("charAt", char_at);
	ctx.add_function("indexOf", index_of);
	ctx.add_function("join", join);
	ctx.add_function("lastIndexOf", last_index_of);
	ctx.add_function("lowerAscii", lower_ascii);
	ctx.add_function("stripPrefix", strip_prefix);
	ctx.add_function("stripSuffix", strip_suffix);
	ctx.add_function("upperAscii", upper_ascii);
	ctx.add_function("trim", trim);
	ctx.add_function("replace", replace);
	ctx.add_function("split", split);
	ctx.add_function("substring", substring);
}

pub fn char_at<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	arg: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let index = arg.load(ftx)?.as_unsigned()?;
	match this.as_ref().chars().nth(index) {
		None => Err(ExecutionError::FunctionError {
			function: "String.charAt".to_owned(),
			message: format!("No index {index} on `{}`", this.as_ref()),
		}),
		Some(c) => Ok(c.to_string().into()),
	}
}

pub fn index_of<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	arg: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let arg: StringValue = arg.load_value(ftx)?;
	match ftx.args.len() {
		1 => match this.as_ref().find(arg.as_ref()) {
			None => Ok((-1).into()),
			Some(idx) => Ok((idx as u64).into()),
		},
		2 => {
			let base_value: Value = ftx.arg(1)?;
			let base = base_value.as_unsigned()?;
			let this_str = this.as_ref();
			if base >= this_str.len() {
				return Ok((-1).into());
			}
			match this_str[base..].find(arg.as_ref()) {
				None => Ok((-1).into()),
				Some(idx) => Ok(Value::UInt((base + idx) as u64)),
			}
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.indexOf".to_owned(),
			message: format!("Expects 2 arguments at most, got `{}`!", ftx.args.len()),
		}),
	}
}

pub fn last_index_of<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	arg: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let arg: StringValue = arg.load_value(ftx)?;
	match ftx.args.len() {
		1 => match this.as_ref().rfind(arg.as_ref()) {
			None => Ok((-1).into()),
			Some(idx) => Ok((idx as u64).into()),
		},
		2 => {
			let base_value: Value = ftx.arg(1)?;
			let base = base_value.as_unsigned()?;
			let this_str = this.as_ref();
			if base >= this_str.len() {
				return Ok((-1).into());
			}
			match this_str[base..].rfind(arg.as_ref()) {
				None => Ok((-1).into()),
				Some(idx) => Ok(Value::UInt(idx as u64)),
			}
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.lastIndexOf".to_owned(),
			message: format!("Expects 2 arguments at most, got `{}`!", ftx.args.len()),
		}),
	}
}

pub fn join<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let this = this.load_value(ftx)?;
	let this = match this {
		Value::List(list) => list,
		other => {
			return Err(ExecutionError::FunctionError {
				function: "List.join".to_owned(),
				message: format!("Expects receiver to be a List, got `{:?}`", other),
			});
		},
	};
	let separator_value = if ftx.args.is_empty() {
		None
	} else {
		Some(ftx.arg(0)?)
	};
	let separator = separator_value
		.as_ref()
		.map(|sep: &StringValue| sep.as_ref())
		.unwrap_or("");
	Ok(
		this
			.as_ref()
			.iter()
			.map(|v| match v {
				Value::String(s) => Ok(s.as_ref().to_string()),
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

pub fn lower_ascii<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	Ok(this.as_ref().to_ascii_lowercase().into())
}

pub fn strip_prefix<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	prefix: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let prefix: StringValue = prefix.load_value(ftx)?;
	let stripped = this
		.as_ref()
		.strip_prefix(prefix.as_ref())
		.unwrap_or(this.as_ref());
	Ok(stripped.to_string().into())
}

pub fn strip_suffix<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	suffix: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let suffix: StringValue = suffix.load_value(ftx)?;
	let stripped = this
		.as_ref()
		.strip_suffix(suffix.as_ref())
		.unwrap_or(this.as_ref());
	Ok(stripped.to_string().into())
}

pub fn upper_ascii<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	Ok(this.as_ref().to_ascii_uppercase().into())
}

pub fn trim<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	Ok(this.as_ref().trim().to_string().into())
}

pub fn replace<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	from: Argument,
	to: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let from: StringValue = from.load_value(ftx)?;
	let to: StringValue = to.load_value(ftx)?;
	match ftx.args.len() {
		2 => Ok(this.as_ref().replace(from.as_ref(), to.as_ref()).into()),
		3 => {
			let n_value: Value = ftx.arg(2)?;
			let n = n_value.as_signed()?;
			if n < 0 {
				Ok(this.as_ref().replace(from.as_ref(), to.as_ref()).into())
			} else {
				Ok(
					this
						.as_ref()
						.replacen(from.as_ref(), to.as_ref(), n as usize)
						.into(),
				)
			}
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.replace".to_owned(),
			message: format!("Expects 2 or 3 arguments, got {}!", ftx.args.len()),
		}),
	}
}

pub fn split<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	sep: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let sep: StringValue = sep.load_value(ftx)?;
	match ftx.args.len() {
		1 => Ok(
			this
				.as_ref()
				.split(sep.as_ref())
				.map(|s| Value::String(s.to_owned().into()))
				.collect::<Vec<Value>>()
				.into(),
		),
		2 => {
			let pos_value: Value = ftx.arg(1)?;
			let pos = pos_value.as_signed()?;
			let split = if pos < 0 {
				this
					.as_ref()
					.split(sep.as_ref())
					.map(|s| Value::String(s.to_owned().into()))
					.collect::<Vec<Value>>()
			} else {
				this
					.as_ref()
					.splitn(pos as usize, sep.as_ref())
					.map(|s| Value::String(s.to_owned().into()))
					.collect::<Vec<Value>>()
			};
			Ok(split.into())
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.split".to_owned(),
			message: format!("Expects at most 2 arguments, got {}!", ftx.args.len()),
		}),
	}
}

pub fn substring<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	start: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let start = start.load(ftx)?.as_unsigned()?;
	match ftx.args.len() {
		1 => {
			let end = this.as_ref().chars().count();
			if end < start {
				return Err(ExecutionError::FunctionError {
					function: "String.substring".to_string(),
					message: format!("Can't have end be before the start: `{end} < {start}"),
				});
			}
			Ok(
				this
					.as_ref()
					.chars()
					.skip(start)
					.take(end - start)
					.collect::<String>()
					.into(),
			)
		},
		2 => {
			let end = ftx.value(1)?.as_unsigned()?;
			if end < start {
				return Err(ExecutionError::FunctionError {
					function: "String.substring".to_string(),
					message: format!("Can't have end be before the start: `{end} < {start}"),
				});
			}
			Ok(
				this
					.as_ref()
					.chars()
					.skip(start)
					.take(end - start)
					.collect::<String>()
					.into(),
			)
		},
		_ => Err(ExecutionError::FunctionError {
			function: "String.substring".to_owned(),
			message: format!("Expects at most 2 arguments, got {}!", ftx.args.len()),
		}),
	}
}
#[cfg(test)]
mod tests {
	use cel::{Context, Program};
	use serde_json::json;

	use crate::insert_all;

	fn eval(expr: &str) -> serde_json::Value {
		let prog = Program::compile(expr).unwrap_or_else(|_| panic!("failed to compile: {}", expr));
		let mut c = Context::default();
		insert_all(&mut c);
		prog
			.execute(&c)
			.unwrap_or_else(|_| panic!("{expr}"))
			.json()
			.unwrap()
	}

	#[test]
	fn extended_string_fn() {
		assert_eq!(eval("'abc'.charAt(1)"), json!("b"));

		assert_eq!(eval("'hello mellow'.indexOf('')"), json!(0));
		assert_eq!(eval("'hello mellow'.indexOf('ello')"), json!(1));
		assert_eq!(eval("'hello mellow'.indexOf('jello')"), json!((-1)));
		assert_eq!(eval("'hello mellow'.indexOf('', 2)"), json!(2));
		assert_eq!(eval("'hello mellow'.indexOf('ello', 20)"), json!((-1)));

		assert_eq!(eval("'hello mellow'.lastIndexOf('')"), json!(12));
		assert_eq!(eval("'hello mellow'.lastIndexOf('ello')"), json!(7));
		assert_eq!(eval("'hello mellow'.lastIndexOf('jello')"), json!((-1)));
		assert_eq!(eval("'hello mellow'.lastIndexOf('ello', 6)"), json!(1));
		assert_eq!(eval("'hello mellow'.lastIndexOf('ello', 20)"), json!((-1)));

		assert_eq!(eval("['hello', 'mellow'].join()"), json!("hellomellow"));
		assert_eq!(eval("[].join()"), json!(""));
		assert_eq!(eval("['hello', 'mellow'].join(' ')"), json!("hello mellow"));

		assert_eq!(eval("'TacoCat'.lowerAscii()"), json!("tacocat"));
		assert_eq!(eval("'TacoCÆt Xii'.lowerAscii()"), json!("tacocÆt xii"));

		assert_eq!(eval("'hello'.startsWith('he')"), json!(true));
		assert_eq!(eval("'hello'.startsWith('lo')"), json!(false));
		assert_eq!(eval("'hello'.startsWith('')"), json!(true));

		assert_eq!(eval("'hello'.endsWith('lo')"), json!(true));
		assert_eq!(eval("'hello'.endsWith('he')"), json!(false));
		assert_eq!(eval("'hello'.endsWith('')"), json!(true));

		assert_eq!(eval("'hello'.stripPrefix('he')"), json!("llo"));
		assert_eq!(eval("'hello'.stripPrefix('hi')"), json!("hello"));
		assert_eq!(eval("'hello'.stripPrefix('')"), json!("hello"));

		assert_eq!(eval("'hello'.stripSuffix('lo')"), json!("hel"));
		assert_eq!(eval("'hello'.stripSuffix('hi')"), json!("hello"));
		assert_eq!(eval("'hello'.stripSuffix('')"), json!("hello"));

		assert_eq!(eval("'TacoCat'.upperAscii()"), json!("TACOCAT"));
		assert_eq!(eval("'TacoCÆt Xii'.upperAscii()"), json!("TACOCÆT XII"));

		assert_eq!(eval("'  trim\\n    '.trim()"), json!("trim"));

		assert_eq!(
			eval("'hello hello'.replace('he', 'we')"),
			json!("wello wello")
		);
		assert_eq!(
			eval("'hello hello'.replace('he', 'we', -1)"),
			json!("wello wello")
		);
		assert_eq!(
			eval("'hello hello'.replace('he', 'we', 1)"),
			json!("wello hello")
		);
		assert_eq!(
			eval("'hello hello'.replace('he', 'we', 0)"),
			json!("hello hello")
		);
		assert_eq!(
			eval("'hello hello'.replace('', '_')"),
			json!("_h_e_l_l_o_ _h_e_l_l_o_")
		);
		assert_eq!(eval("'hello hello'.replace('h', '')"), json!("ello ello"));

		assert_eq!(
			eval("'hello hello hello'.split(' ')"),
			json!(vec!["hello", "hello", "hello"])
		);
		assert_eq!(
			eval("'hello hello hello'.split(' ', 0)"),
			json!(Vec::<String>::new())
		);
		assert_eq!(
			eval("'hello hello hello'.split(' ', 1)"),
			json!(vec!["hello hello hello"])
		);
		assert_eq!(
			eval("'hello hello hello'.split(' ', 2)"),
			json!(vec!["hello", "hello hello"])
		);
		assert_eq!(
			eval("'hello hello hello'.split(' ', -1)"),
			json!(vec!["hello", "hello", "hello"])
		);

		assert_eq!(eval("'tacocat'.substring(4)"), json!("cat"));
		assert_eq!(eval("'tacocat'.substring(0, 4)"), json!("taco"));
		assert_eq!(eval("'ta©o©αT'.substring(2, 6)"), json!("©o©α"));
	}
}
