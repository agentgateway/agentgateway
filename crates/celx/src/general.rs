use std::sync::Arc;

use ::cel::extractors::{Argument, This};
use ::cel::objects::{MapValue, StringValue, ValueType};
use ::cel::{Context, FunctionContext, ResolveResult, Value};
use cel::ExecutionError;
use cel::context::{SingleVarResolver, VariableResolver};
use cel::objects::KeyRef;
use rand::random_range;
use serde::Deserializer;
use uuid::Uuid;

pub fn insert_all(ctx: &mut Context) {
	// Custom to agentgateway
	ctx.add_function("json", json_parse);
	ctx.add_function("jsonField", json_parse_field);
	ctx.add_function("to_json", to_json);
	// Keep old and new name for compatibility
	ctx.add_function("toJson", to_json);
	ctx.add_function("with", with);
	ctx.add_function("mapValues", map_values);
	ctx.add_function("merge", map_merge);
	ctx.add_function("variables", variables);
	ctx.add_function("random", random);
	ctx.add_function("default", default);
	ctx.add_function("regexReplace", regex_replace);
	ctx.add_function("fail", fail);
	ctx.add_function("uuid", uuid_generate);

	// Support legacy and modern name
	ctx.add_function("base64Encode", base64_encode);
	ctx.add_function("base64Decode", base64_decode);
	ctx.add_qualified_function("base64", "encode", base64_encode);
	ctx.add_qualified_function("base64", "decode", base64_decode);
}

pub fn base64_encode<'a>(ftx: &mut FunctionContext<'a, '_>, v: Argument) -> ResolveResult<'a> {
	// The Go library requires bytes, but we accept strings too.
	let v = v.load(ftx)?.always_materialize_owned();
	use base64::Engine;
	Ok(
		base64::prelude::BASE64_STANDARD
			.encode(v.as_bytes_pre_materialized()?)
			.into(),
	)
}

pub fn base64_decode<'a>(ftx: &mut FunctionContext<'a, '_>, v: Argument) -> ResolveResult<'a> {
	// The Go library requires strings, but we accept bytes too.
	let v = v.load(ftx)?.always_materialize_owned();
	use base64::Engine;
	base64::prelude::BASE64_STANDARD
		.decode(v.as_bytes_pre_materialized()?)
		.map(|v| v.into())
		.map_err(|e| ftx.error(e))
}

fn with<'a, 'rf, 'b>(
	ftx: &'b mut FunctionContext<'a, 'rf>,
	this: This,
	ident: Argument,
	expr: Argument,
) -> ResolveResult<'a> {
	let this: Value<'a> = this.load_unmaterialized(ftx)?;
	let ident = ident.load_identifier(ftx)?;
	let expr = expr.load_expression(ftx)?;
	let x: &'rf dyn VariableResolver<'a> = ftx.vars();
	let resolver = SingleVarResolver::<'a, 'rf>::new(x, ident, this);
	let v = Value::resolve(expr, ftx.ptx, &resolver)?;
	drop(resolver);
	Ok(v)
}
pub fn variables<'a, 'rf>(ftx: &mut FunctionContext<'a, 'rf>) -> ResolveResult<'a> {
	// Not ideal; we should find a way to dynamically expose
	let keys = [
		"request",
		"response",
		"jwt",
		"apiKey",
		"basicAuth",
		"llm",
		"llmRequest",
		"source",
		"mcp",
		"backend",
		"extauthz",
		"extproc",
	];
	let mut res = vector_map::VecMap::with_capacity(keys.len());
	for k in keys {
		if let Some(v) = ftx.variables.resolve(k) {
			res.insert(KeyRef::String((*k).into()), v);
		}
	}
	Value::Map(MapValue::Borrow(res)).into()
}

fn map_values<'a, 'rf, 'b>(
	ftx: &'b mut FunctionContext<'a, 'rf>,
	this: This,
	ident: Argument,
	expr: Argument,
) -> ResolveResult<'a> {
	let this: Value<'a> = this.load_value(ftx)?;
	let ident = ident.load_identifier(ftx)?;
	let expr = expr.load_expression(ftx)?;
	let x: &'rf dyn VariableResolver<'a> = ftx.vars();
	match this {
		Value::Map(map) => {
			let mut res = vector_map::VecMap::with_capacity(map.len());
			for k in map.iter_keys() {
				let v = map.get(&k).unwrap().clone();
				let resolver = SingleVarResolver::<'a, 'rf>::new(x, ident, v);
				let value = Value::resolve(expr, ftx.ptx, &resolver)?;
				res.insert(k.clone(), value.as_static());
			}

			Value::Map(MapValue::Borrow(res))
		},
		_ => return Err(this.error_expected_type(ValueType::Map)),
	}
	.into()
}

pub fn map_merge<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	other: Argument,
) -> ResolveResult<'a> {
	let this: Value = this.load_value(ftx)?;
	let other: Value = other.load_value(ftx)?;
	let this = must_map(this)?;
	let other = must_map(other)?;
	let nv = this.iter_owned().chain(other.iter_owned()).collect();
	Value::Map(MapValue::Owned(Arc::new(nv))).into()
}

fn must_map(v: Value) -> Result<MapValue, cel::ExecutionError> {
	match v {
		Value::Map(map) => Ok(map),
		_ => Err(v.error_expected_type(ValueType::Map)),
	}
}

fn fail<'a>(ftx: &mut FunctionContext<'a, '_>, v: Argument) -> ResolveResult<'a> {
	let v: StringValue = v.load_value(ftx)?;
	Err(ftx.error(format!("fail() called: {}", v.as_ref())))
}

fn json_parse<'a>(ftx: &mut FunctionContext<'a, '_>, v: Argument) -> ResolveResult<'a> {
	let v: Value = v.load_value(ftx)?;
	let sv = match v {
		Value::String(b) => serde_json::from_str(b.as_ref()),
		Value::Bytes(b) => serde_json::from_slice(b.as_ref()),
		_ => return Err(ftx.error(format!("invalid type {}", v.type_of()))),
	};
	let sv: serde_json::Value = sv.map_err(|e| ftx.error(e))?;
	cel::to_value(sv).map_err(|e| ftx.error(e))
}

fn to_json<'a>(ftx: &mut FunctionContext<'a, '_>, v: Argument) -> ResolveResult<'a> {
	let v: Value = v.load_value(ftx)?;
	let pj = v.json().map_err(|e| ftx.error(e))?;
	Ok(Value::String(
		serde_json::to_string(&pj).map_err(|e| ftx.error(e))?.into(),
	))
}

pub fn regex_replace<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	regex: Argument,
	replacement: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_value(ftx)?;
	let regex: StringValue = regex.load_value(ftx)?;
	let replacement: StringValue = replacement.load_value(ftx)?;
	match regex::Regex::new(regex.as_ref()) {
		Ok(re) => Ok(
			re.replace(this.as_ref(), replacement.as_ref())
				.to_string()
				.into(),
		),
		Err(err) => Err(ftx.error(format!("'{}' not a valid regex:\n{err}", regex.as_ref()))),
	}
}

fn uuid_generate<'a>(_: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	Ok(Uuid::new_v4().to_string().into())
}

fn random<'a>(_: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	Ok(random_range(0.0..=1.0).into())
}

fn default<'a>(ftx: &mut FunctionContext<'a, '_>, exp: Argument, d: Argument) -> ResolveResult<'a> {
	// We determine if a type has a property by attempting to resolve it.
	// If we get a NoSuchKey error, then we know the property does not exist
	let exp = exp.load_expression(ftx)?;
	let resolved = match Value::resolve(exp, ftx.ptx, ftx.vars()) {
		Ok(Value::Null) => None,
		Ok(v) => Some(v),
		Err(err) => match err {
			cel::ExecutionError::NoSuchKey(_) => None,
			cel::ExecutionError::UndeclaredReference(_) => None,
			_ => return Err(err),
		},
	};
	match resolved {
		Some(v) => Ok(v),
		None => Ok(d.load_unmaterialized(ftx)?),
	}
}

mod json_field {
	use std::fmt;

	use serde::de::{Error, IgnoredAny, MapAccess, Visitor};

	impl FieldExtractor {
		pub fn new(field_name: &str) -> FieldExtractor {
			FieldExtractor(field_name.to_string())
		}
	}

	pub struct FieldExtractor(String);

	impl<'de> Visitor<'de> for FieldExtractor {
		type Value = Option<cel::Value<'static>>;

		fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
			formatter.write_str("a map")
		}

		fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
		where
			A: MapAccess<'de>,
		{
			let mut val = None;
			while let Some(key) = map.next_key::<String>()? {
				if key == self.0 {
					let value = map.next_value::<serde_json::Value>()?;
					let value = cel::to_value(value).map_err(A::Error::custom)?;
					val = Some(value);
				} else {
					map.next_value::<IgnoredAny>()?;
				}
				// Do not recurse into the values, we only allow top level access right now
			}

			Ok(val)
		}
	}
}
fn json_parse_field<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	v: Argument,
	k: Argument,
) -> ResolveResult<'a> {
	let v = v.load_value(ftx)?;
	let k: StringValue = k.load_value(ftx)?;
	let pv = match v {
		Value::String(b) => {
			let mut d = serde_json::de::Deserializer::from_str(b.as_ref());
			d.deserialize_map(json_field::FieldExtractor::new(&k))
				.map_err(|e| ftx.error(e))?
		},
		Value::Bytes(b) => {
			let mut d = serde_json::de::Deserializer::from_slice(b.as_ref());
			d.deserialize_map(json_field::FieldExtractor::new(&k))
				.map_err(|e| ftx.error(e))?
		},
		_ => return Err(ftx.error(format!("invalid type {}", v.type_of()))),
	};

	let pv = pv.ok_or_else(|| ExecutionError::NoSuchKey(Arc::from(k.as_ref())))?;
	Ok(pv)
}
