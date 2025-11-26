use std::collections::HashMap;
use std::string::ToString;
use std::sync::Arc;

use ::cel::extractors::{Identifier, This};
use ::cel::objects::{Map, ValueType};
use ::cel::parser::Expression;
use ::cel::{Context, FunctionContext, ResolveResult, Value};
use rand::random_range;
use serde::ser::Error;
use serde::{Serialize, Serializer};

pub fn insert_all(ctx: &mut Context<'_>) {
	// Custom to agentgateway
	ctx.add_function("json", json_parse);
	ctx.add_function("to_json", to_json);
	// Keep old and new name for compatibility
	ctx.add_function("toJson", to_json);
	ctx.add_function("with", with);
	ctx.add_function("flatten", flatten);
	ctx.add_function("flatten_recursive", flatten_recursive);
	// Keep old and new name for compatibility
	ctx.add_function("flattenRecursive", flatten_recursive);
	ctx.add_function("mapValues", map_values);
	ctx.add_function("merge", map_merge);
	ctx.add_function("variables", variables);
	ctx.add_function("random", || random_range(0.0..=1.0));
	ctx.add_function("default", default);
	ctx.add_function("regexReplace", regex_replace);
	ctx.add_function("fail", fail);

	// Using the go name, base64.encode is blocked by https://github.com/cel-rust/cel-rust/issues/103 (namespacing)
	ctx.add_function("base64Encode", base64_encode);
	ctx.add_function("base64Decode", base64_decode);
}

pub fn base64_encode(This(this): This<Arc<String>>) -> String {
	use base64::Engine;
	base64::prelude::BASE64_STANDARD.encode(this.as_bytes())
}

pub fn base64_decode(ftx: &FunctionContext, This(this): This<Arc<String>>) -> ResolveResult {
	use base64::Engine;
	base64::prelude::BASE64_STANDARD
		.decode(this.as_ref())
		.map(|v| Value::Bytes(Arc::new(v)))
		.map_err(|e| ftx.error(e))
}

fn with(
	ftx: &FunctionContext,
	This(this): This<Value>,
	ident: Identifier,
	expr: Expression,
) -> ResolveResult {
	let mut ptx = ftx.ptx.new_inner_scope();
	ptx.add_variable_from_value(&ident, this);
	ptx.resolve(&expr)
}

#[derive(Clone, Debug)]
pub enum FlattenSignal {
	Map(Map),
	MapRecursive(Map),
	List(Arc<Vec<Value>>),
	ListRecursive(Arc<Vec<Value>>),
}
impl Serialize for FlattenSignal {
	fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		Err(S::Error::custom("cannot serialize FlattenSignal"))
	}
}
impl Eq for FlattenSignal {}
impl PartialEq for FlattenSignal {
	fn eq(&self, _: &Self) -> bool {
		false
	}
}
crate::impl_opaque!(FlattenSignal, "flatten_signal");
impl FlattenSignal {
	pub fn from_value(v: &Value) -> Option<FlattenSignal> {
		let Value::Opaque(s) = v else {
			return None;
		};
		Some(crate::helpers::cast::<Self>(s).ok()?.clone())
	}
}

fn flatten(ftx: &FunctionContext, v: Value) -> ResolveResult {
	let res = match v {
		Value::List(l) => Value::Opaque(Arc::new(FlattenSignal::List(l))),
		Value::Map(m) => Value::Opaque(Arc::new(FlattenSignal::Map(m))),
		_ => {
			return ftx.error("flatten only works on Map or List").into();
		},
	};
	res.into()
}

fn flatten_recursive(ftx: &FunctionContext, v: Value) -> ResolveResult {
	let res = match v {
		Value::List(l) => Value::Opaque(Arc::new(FlattenSignal::ListRecursive(l))),
		Value::Map(m) => Value::Opaque(Arc::new(FlattenSignal::MapRecursive(m))),
		_ => {
			return ftx.error("flatten only works on Map or List").into();
		},
	};
	res.into()
}

fn variables(ftx: &FunctionContext) -> ResolveResult {
	fn variables_inner<'context>(
		ctx: &'context Context<'context>,
	) -> HashMap<cel::objects::Key, Value> {
		match ctx {
			Context::Root { variables, .. } => variables
				.clone()
				.iter()
				.map(|(k, v)| (cel::objects::Key::from(k.as_str()), v.clone()))
				.collect(),
			Context::Child {
				parent, variables, ..
			} => {
				let mut base = variables_inner(parent);
				base.extend(
					variables
						.iter()
						.map(|(k, v)| (cel::objects::Key::from(k.as_str()), v.clone())),
				);
				base
			},
		}
	}
	Value::Map(Map {
		map: Arc::new(variables_inner(ftx.ptx)),
	})
	.into()
}

pub fn map_values(
	ftx: &FunctionContext,
	This(this): This<Value>,
	ident: Identifier,
	expr: Expression,
) -> ResolveResult {
	match this {
		Value::Map(map) => {
			let mut res = HashMap::with_capacity(map.map.len());
			let mut ptx = ftx.ptx.new_inner_scope();
			for (key, val) in map.map.as_ref() {
				ptx.add_variable_from_value(ident.clone(), val.clone());
				let value = ptx.resolve(&expr)?;
				res.insert(key.clone(), value);
			}
			Value::Map(Map { map: Arc::new(res) })
		},
		_ => return Err(this.error_expected_type(ValueType::Map)),
	}
	.into()
}

pub fn map_merge(This(this): This<Value>, other: Value) -> ResolveResult {
	let this = must_map(this)?;
	let other = must_map(other)?;
	let mut nv = Arc::unwrap_or_clone(this.map);
	nv.extend(Arc::unwrap_or_clone(other.map));
	Value::Map(Map { map: Arc::new(nv) }).into()
}

fn must_map(v: Value) -> Result<Map, cel::ExecutionError> {
	match v {
		Value::Map(map) => Ok(map),
		_ => Err(v.error_expected_type(ValueType::Map)),
	}
}

fn fail(ftx: &FunctionContext, v: Arc<String>) -> ResolveResult {
	Err(ftx.error(format!("fail() called: {v}")))
}

fn json_parse(ftx: &FunctionContext, v: Value) -> ResolveResult {
	let sv = match v {
		Value::String(b) => serde_json::from_str(b.as_str()),
		Value::Bytes(b) => serde_json::from_slice(b.as_ref()),
		_ => return Err(ftx.error(format!("invalid type {}", v.type_of()))),
	};
	let sv: serde_json::Value = sv.map_err(|e| ftx.error(e))?;
	cel::to_value(sv).map_err(|e| ftx.error(e))
}

fn to_json(ftx: &FunctionContext, v: Value) -> ResolveResult {
	let pj = v.json().map_err(|e| ftx.error(e))?;
	Ok(Value::String(Arc::new(
		serde_json::to_string(&pj).map_err(|e| ftx.error(e))?,
	)))
}

pub fn regex_replace(
	ftx: &FunctionContext,
	This(this): This<Arc<String>>,
	regex: Arc<String>,
	replacement: Arc<String>,
) -> Result<Arc<String>, cel::ExecutionError> {
	match regex::Regex::new(&regex) {
		Ok(re) => Ok(Arc::new(
			re.replace(&this, replacement.as_str()).to_string(),
		)),
		Err(err) => Err(ftx.error(format!("'{regex}' not a valid regex:\n{err}"))),
	}
}

fn default(ftx: &FunctionContext, exp: Expression, d: Value) -> ResolveResult {
	fn has(ftx: &FunctionContext, exp: Expression) -> Result<Option<Value>, cel::ExecutionError> {
		// We determine if a type has a property by attempting to resolve it.
		// If we get a NoSuchKey error, then we know the property does not exist
		Ok(match ftx.resolve(exp) {
			Ok(Value::Null) => None,
			Ok(v) => Some(v),
			Err(err) => match err {
				cel::ExecutionError::NoSuchKey(_) => None,
				cel::ExecutionError::UndeclaredReference(_) => None,
				_ => return Err(err),
			},
		})
	}
	Ok(has(ftx, exp)?.unwrap_or(d))
}
