use std::cmp::Ordering;

use cel::extractors::Argument;
use cel::objects::{ListValue, ValueType};
use cel::{Context, ExecutionError, FunctionContext, ResolveResult, Value};

pub fn insert_all(ctx: &mut Context) {
	ctx.add_qualified_function("math", "least", least);
	ctx.add_qualified_function("math", "greatest", greatest);
	ctx.add_qualified_function("math", "ceil", ceil);
	ctx.add_qualified_function("math", "floor", floor);
	ctx.add_qualified_function("math", "round", round);
	ctx.add_qualified_function("math", "trunc", trunc);
	ctx.add_qualified_function("math", "isInf", is_inf);
	ctx.add_qualified_function("math", "isNaN", is_nan);
	ctx.add_qualified_function("math", "isFinite", is_finite);
	ctx.add_qualified_function("math", "abs", abs);
	ctx.add_qualified_function("math", "sign", sign);
	ctx.add_qualified_function("math", "sqrt", sqrt);
	ctx.add_qualified_function("math", "bitAnd", bit_and);
	ctx.add_qualified_function("math", "bitOr", bit_or);
	ctx.add_qualified_function("math", "bitXor", bit_xor);
	ctx.add_qualified_function("math", "bitNot", bit_not);
	ctx.add_qualified_function("math", "bitShiftLeft", bit_shift_left);
	ctx.add_qualified_function("math", "bitShiftRight", bit_shift_right);
}

fn least<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	extreme(ftx, Ordering::Less, "math.least")
}

fn greatest<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	extreme(ftx, Ordering::Greater, "math.greatest")
}

fn extreme<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	keep: Ordering,
	function: &str,
) -> ResolveResult<'a> {
	if ftx.args.is_empty() {
		return Err(ftx.error(format!("{function}() requires at least one argument")));
	}
	if ftx.args.len() == 1 {
		let value = ftx.value(0)?;
		if let Value::List(values) = value {
			return extreme_list(ftx, &values, keep, function);
		}
		ensure_numeric(&value, ftx, function)?;
		return Ok(value);
	}

	let mut values = ftx.value_iter();
	let mut acc = values.next().transpose()?.expect("checked non-empty args");
	ensure_numeric(&acc, ftx, function)?;
	for value in values {
		let value = value?;
		ensure_numeric(&value, ftx, function)?;
		acc = extreme_pair(acc, value, keep, function)?;
	}
	Ok(acc)
}

fn extreme_list<'a>(
	ftx: &FunctionContext<'a, '_>,
	values: &ListValue<'a>,
	keep: Ordering,
	function: &str,
) -> ResolveResult<'a> {
	let mut iter = values.as_ref().iter();
	let mut acc = iter
		.next()
		.ok_or_else(|| ftx.error(format!("{function}(list) argument must not be empty")))?
		.clone()
		.always_materialize_owned();
	ensure_numeric(&acc, ftx, function)?;
	for value in iter {
		let value = value.clone().always_materialize_owned();
		ensure_numeric(&value, ftx, function)?;
		acc = extreme_pair(acc, value, keep, function)?;
	}
	Ok(acc)
}

fn extreme_pair<'a>(
	left: Value<'a>,
	right: Value<'a>,
	keep: Ordering,
	function: &str,
) -> ResolveResult<'a> {
	match left.partial_cmp(&right) {
		Some(ordering) if ordering == keep => Ok(left),
		Some(_) => Ok(right),
		None => Err(ExecutionError::FunctionError {
			function: function.to_owned(),
			message: format!("{:?} can not be compared to {:?}", left, right),
		}),
	}
}

fn ensure_numeric(
	value: &Value<'_>,
	ftx: &FunctionContext<'_, '_>,
	function: &str,
) -> Result<(), ExecutionError> {
	match value {
		Value::Int(_) | Value::UInt(_) | Value::Float(_) => Ok(()),
		_ => Err(ftx.error(format!(
			"{function} arguments must be numeric, got {}",
			value.type_of().as_str()
		))),
	}
}

fn ceil<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Float(value.load(ftx)?.as_float(ftx)?.ceil()))
}

fn floor<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Float(value.load(ftx)?.as_float(ftx)?.floor()))
}

fn round<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Float(value.load(ftx)?.as_float(ftx)?.round()))
}

fn trunc<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Float(value.load(ftx)?.as_float(ftx)?.trunc()))
}

fn is_inf<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Bool(value.load(ftx)?.as_float(ftx)?.is_infinite()))
}

fn is_nan<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Bool(value.load(ftx)?.as_float(ftx)?.is_nan()))
}

fn is_finite<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Bool(value.load(ftx)?.as_float(ftx)?.is_finite()))
}

fn abs<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	match value.load(ftx)? {
		Value::Int(v) => v
			.checked_abs()
			.map(Value::Int)
			.ok_or_else(|| ftx.error("integer overflow")),
		Value::UInt(v) => Ok(Value::UInt(v)),
		Value::Float(v) => Ok(Value::Float(v.abs())),
		value => Err(value.error_expected_type(ValueType::Int)),
	}
}

fn sign<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(match value.load(ftx)? {
		Value::Int(v) => Value::Int(v.signum()),
		Value::UInt(0) => Value::UInt(0),
		Value::UInt(_) => Value::UInt(1),
		Value::Float(v) if v.is_nan() => Value::Float(v),
		Value::Float(v) => Value::Float(if v > 0.0 {
			1.0
		} else if v < 0.0 {
			-1.0
		} else {
			0.0
		}),
		value => return Err(value.error_expected_type(ValueType::Float)),
	})
}

fn sqrt<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	Ok(Value::Float(value.load(ftx)?.as_number(ftx)?.sqrt()))
}

fn bit_and<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	left: Argument,
	right: Argument,
) -> ResolveResult<'a> {
	bit_pair(ftx, left, right, |l, r| l & r, |l, r| l & r)
}

fn bit_or<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	left: Argument,
	right: Argument,
) -> ResolveResult<'a> {
	bit_pair(ftx, left, right, |l, r| l | r, |l, r| l | r)
}

fn bit_xor<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	left: Argument,
	right: Argument,
) -> ResolveResult<'a> {
	bit_pair(ftx, left, right, |l, r| l ^ r, |l, r| l ^ r)
}

fn bit_pair<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	left: Argument,
	right: Argument,
	int_op: impl FnOnce(i64, i64) -> i64,
	uint_op: impl FnOnce(u64, u64) -> u64,
) -> ResolveResult<'a> {
	match (left.load(ftx)?, right.load(ftx)?) {
		(Value::Int(left), Value::Int(right)) => Ok(Value::Int(int_op(left, right))),
		(Value::UInt(left), Value::UInt(right)) => Ok(Value::UInt(uint_op(left, right))),
		_ => Err(ExecutionError::NoSuchOverload),
	}
}

fn bit_not<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	match value.load(ftx)? {
		Value::Int(value) => Ok(Value::Int(!value)),
		Value::UInt(value) => Ok(Value::UInt(!value)),
		_ => Err(ExecutionError::NoSuchOverload),
	}
}

fn bit_shift_left<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	value: Argument,
	bits: Argument,
) -> ResolveResult<'a> {
	let bits = load_shift_bits(ftx, bits, "math.bitShiftLeft")?;
	match value.load(ftx)? {
		Value::Int(value) => Ok(Value::Int(value.checked_shl(bits).unwrap_or(0))),
		Value::UInt(value) => Ok(Value::UInt(value.checked_shl(bits).unwrap_or(0))),
		_ => Err(ExecutionError::NoSuchOverload),
	}
}

fn bit_shift_right<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	value: Argument,
	bits: Argument,
) -> ResolveResult<'a> {
	let bits = load_shift_bits(ftx, bits, "math.bitShiftRight")?;
	match value.load(ftx)? {
		Value::Int(value) => Ok(Value::Int(
			((value as u64).checked_shr(bits).unwrap_or(0)) as i64,
		)),
		Value::UInt(value) => Ok(Value::UInt(value.checked_shr(bits).unwrap_or(0))),
		_ => Err(ExecutionError::NoSuchOverload),
	}
}

fn load_shift_bits(
	ftx: &mut FunctionContext<'_, '_>,
	bits: Argument,
	function: &str,
) -> Result<u32, ExecutionError> {
	let Value::Int(bits) = bits.load(ftx)? else {
		return Err(ExecutionError::NoSuchOverload);
	};
	if bits < 0 {
		return Err(ftx.error(format!("{function}() negative offset: {bits}")));
	}
	Ok(bits.try_into().unwrap_or(u32::MAX))
}

trait MathValue {
	fn as_float(&self, ftx: &FunctionContext<'_, '_>) -> Result<f64, ExecutionError>;
	fn as_number(&self, ftx: &FunctionContext<'_, '_>) -> Result<f64, ExecutionError>;
}

impl MathValue for Value<'_> {
	fn as_float(&self, ftx: &FunctionContext<'_, '_>) -> Result<f64, ExecutionError> {
		match self {
			Value::Float(v) => Ok(*v),
			value => Err(ftx.error(format!("expected double, got {}", value.type_of().as_str()))),
		}
	}

	fn as_number(&self, ftx: &FunctionContext<'_, '_>) -> Result<f64, ExecutionError> {
		match self {
			Value::Int(v) => Ok(*v as f64),
			Value::UInt(v) => Ok(*v as f64),
			Value::Float(v) => Ok(*v),
			value => Err(ftx.error(format!(
				"expected numeric value, got {}",
				value.type_of().as_str()
			))),
		}
	}
}
