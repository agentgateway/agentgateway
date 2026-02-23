use std::cmp::Ordering;
use std::convert::TryInto;
use std::sync::Arc;

use crate::context::{Context, VariableResolver};
use crate::magic::{Argument, FromValue, This};
use crate::objects::{BytesValue, KeyRef, OpaqueValue, OptionalValue, StringValue, Value};
use crate::parser::Expression;
use crate::{ExecutionError, ResolveResult};

type Result<T> = std::result::Result<T, ExecutionError>;

/// `FunctionContext` is a context object passed to functions when they are called.
///
/// It contains references to the target object (if the function is called as
/// a method), the program context ([`Context`]) which gives functions access
/// to variables, and the arguments to the function call.
#[derive(Clone)]
pub struct FunctionContext<'vars, 'rf> {
	pub name: &'vars str,
	pub this: Option<Value<'vars>>,
	pub ptx: &'vars Context,
	pub args: &'vars [Expression],
	pub arg_idx: usize,
	pub variables: &'rf dyn VariableResolver<'vars>,
}

impl<'a, 'vars: 'a, 'rf> FunctionContext<'vars, 'rf> {
	pub fn vars(&self) -> &'rf dyn VariableResolver<'vars> {
		self.variables
	}
	pub fn this<T: FromValue<'a>>(&self) -> Result<T> {
		if let Some(ref this) = self.this {
			Ok(T::from_value(this)?)
		} else {
			Err(ExecutionError::missing_argument_or_target())
		}
	}
	pub fn this_unmaterialized(&self) -> Result<Value<'a>> {
		if let Some(ref this) = self.this {
			Ok(this.clone())
		} else {
			Err(ExecutionError::missing_argument_or_target())
		}
	}
	pub fn this_or_arg<T: FromValue<'a>>(&self) -> Result<T> {
		match self.this() {
			Ok(val) => Ok(val),
			Err(_e) => self.arg(0),
		}
	}
	pub fn this_value(&self) -> Result<Value<'a>> {
		self.this::<Value>().map(|v| v.always_materialize_owned())
	}
	pub fn this_or_arg_value(&self) -> Result<Value<'a>> {
		self
			.this_or_arg::<Value>()
			.map(|v| v.always_materialize_owned())
	}
	pub fn value(&self, index: usize) -> Result<Value<'a>> {
		self
			.value_unmaterialized(index)
			.map(|v| v.always_materialize_owned())
	}
	pub fn value_unmaterialized(&self, index: usize) -> Result<Value<'a>> {
		let arg = self
			.args
			.get(index)
			.ok_or(ExecutionError::invalid_argument_count(
				index + 1,
				self.args.len(),
			))?;
		Value::resolve(arg, self.ptx, self.variables)
	}

	pub fn arg<T: FromValue<'a>>(&self, index: usize) -> Result<T> {
		let v = self.value(index)?;
		T::from_value(&v)
	}

	pub fn ident(&self, index: usize) -> Result<&'a str> {
		match &self.expr(index)?.expr {
			Expr::Ident(ident) => Ok(ident),
			expr => Err(ExecutionError::UnexpectedType {
				got: expr.type_name(),
				want: "identifier",
			}),
		}
	}

	pub fn value_iter(&self) -> impl Iterator<Item = Result<Value<'a>>> + use<'a, '_, 'vars> {
		self
			.args
			.iter()
			.map(|e| Value::resolve(e, self.ptx, self.variables).map(|v| v.always_materialize_owned()))
	}

	pub fn expr_iter(&self) -> impl Iterator<Item = &'a Expression> + use<'a, 'vars> {
		self.args.iter()
	}
	pub fn expr(&self, index: usize) -> Result<&'a Expression> {
		self
			.args
			.get(index)
			.ok_or(ExecutionError::invalid_argument_count(
				index + 1,
				self.args.len(),
			))
	}
}

impl<'vars, 'rf> FunctionContext<'vars, 'rf> {
	pub fn new(
		name: &'vars str,
		this: Option<Value<'vars>>,
		ptx: &'vars Context,
		args: &'vars [Expression],
		variables: &'rf dyn VariableResolver<'vars>,
	) -> Self {
		Self {
			name,
			this,
			ptx,
			args,
			arg_idx: 0,
			variables,
		}
	}

	/// Returns an execution error for the currently execution function.
	pub fn error<M: ToString>(&self, message: M) -> ExecutionError {
		ExecutionError::function_error(self.name, message)
	}
}

/// Calculates the size of either the target, or the provided args depending on how
/// the function is called.
///
/// If called as a method, the target will be used. If called as a function, the
/// first argument will be used.
///
/// The following [`Value`] variants are supported:
/// * [`Value::List`]
/// * [`Value::Map`]
/// * [`Value::String`]
/// * [`Value::Bytes`]
///
/// # Examples
/// ```skip
/// size([1, 2, 3]) == 3
/// ```
/// ```skip
/// 'foobar'.size() == 6
/// ```
pub fn size<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let value = this.load_or_arg_value(ftx)?;
	let size = match value {
		Value::List(l) => l.len(),
		Value::Map(m) => m.len(),
		Value::String(s) => s.as_ref().len(),
		Value::Bytes(b) => b.as_ref().len(),
		value => return Err(ftx.error(format!("cannot determine the size of {value:?}"))),
	};
	Ok(Value::Int(size as i64))
}

/// Returns true if the target contains the provided argument. The actual behavior
/// depends mainly on the type of the target.
///
/// The following [`Value`] variants are supported:
/// * [`Value::List`] - Returns true if the list contains the provided value.
/// * [`Value::Map`] - Returns true if the map contains the provided key.
/// * [`Value::String`] - Returns true if the string contains the provided substring.
/// * [`Value::Bytes`] - Returns true if the bytes contain the provided byte.
///
/// # Example
///
/// ## List
/// ```cel
/// [1, 2, 3].contains(1) == true
/// ```
///
/// ## Map
/// ```cel
/// {"a": 1, "b": 2, "c": 3}.contains("a") == true
/// ```
///
/// ## String
/// ```cel
/// "abc".contains("b") == true
/// ```
///
/// ## Bytes
/// ```cel
/// b"abc".contains(b"c") == true
/// ```
pub fn contains<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	arg: Argument,
) -> ResolveResult<'a> {
	// TODO: we could support a non-materialized lookup for a Map + String
	let this = this.load_or_arg_value(ftx)?;
	let arg: Value<'a> = arg.load_value(ftx)?;
	Ok(Value::Bool(match this {
		Value::List(v) => v.as_ref().contains(&arg),
		Value::Map(v) => v.contains_key(
			&KeyRef::try_from(&arg).map_err(|v| ExecutionError::UnsupportedKeyType(v.as_static()))?,
		),
		Value::String(s) => {
			if let Value::String(arg) = arg {
				s.as_ref().contains(arg.as_ref())
			} else {
				false
			}
		},
		Value::Bytes(b) => {
			if let Value::Bytes(arg) = arg {
				let needle = arg.as_ref();
				if needle.is_empty() {
					true
				} else {
					b.as_ref().windows(needle.len()).any(|w| w == needle)
				}
			} else {
				false
			}
		},
		_ => return Err(ftx.error("contains does not support this type")),
	}))
}

// Performs a type conversion on the target. The following conversions are currently
// supported:
// * `string` - Returns a copy of the target string.
// * `timestamp` - Returns the timestamp in RFC3339 format.
// * `duration` - Returns the duration in a string formatted like "72h3m0.5s".
// * `int` - Returns the integer value of the target.
// * `uint` - Returns the unsigned integer value of the target.
// * `float` - Returns the float value of the target.
// * `bytes` - Converts bytes to string using from_utf8_lossy.
pub fn string<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	let this = ftx.this_or_arg_value()?;
	Ok(match this {
		Value::String(v) => Value::String(v.clone()),

		Value::Timestamp(t) => Value::String(t.to_rfc3339().into()),

		Value::Duration(v) => Value::String(crate::duration::format_duration(&v).into()),
		Value::Int(v) => Value::String(v.to_string().into()),
		Value::UInt(v) => Value::String(v.to_string().into()),
		Value::Float(v) => Value::String(v.to_string().into()),
		Value::Bytes(v) => Value::String(StringValue::Owned(Arc::from(
			String::from_utf8_lossy(v.as_ref()).as_ref(),
		))),
		v => return Err(ftx.error(format!("cannot convert {v:?} to string"))),
	})
}
pub fn bytes<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	let value: StringValue = ftx.arg(0)?;
	Ok(Value::Bytes(BytesValue::Owned(value.as_bytes().into())))
}

// Performs a type conversion on the target.
pub fn double<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	let this = ftx.this_or_arg_value()?;
	Ok(match this {
		Value::String(v) => v
			.as_ref()
			.parse::<f64>()
			.map(Value::Float)
			.map_err(|e| ftx.error(format!("string parse error: {e}")))?,
		Value::Float(v) => Value::Float(v),
		Value::Int(v) => Value::Float(v as f64),
		Value::UInt(v) => Value::Float(v as f64),
		v => return Err(ftx.error(format!("cannot convert {v:?} to double"))),
	})
}

// Performs a type conversion on the target.
pub fn uint<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	let this = ftx.this_or_arg_value()?;
	Ok(match this {
		Value::String(v) => v
			.as_ref()
			.parse::<u64>()
			.map(Value::UInt)
			.map_err(|e| ftx.error(format!("string parse error: {e}")))?,
		Value::Float(v) => {
			if v > u64::MAX as f64 || v < u64::MIN as f64 {
				return Err(ftx.error("unsigned integer overflow"));
			}
			Value::UInt(v as u64)
		},
		Value::Int(v) => Value::UInt(
			v.try_into()
				.map_err(|_| ftx.error("unsigned integer overflow"))?,
		),
		Value::UInt(v) => Value::UInt(v),
		v => return Err(ftx.error(format!("cannot convert {v:?} to uint"))),
	})
}

// Performs a type conversion on the target.
pub fn int<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	let this = ftx.this_or_arg_value()?;
	Ok(match this {
		Value::String(v) => v
			.as_ref()
			.parse::<i64>()
			.map(Value::Int)
			.map_err(|e| ftx.error(format!("string parse error: {e}")))?,
		Value::Float(v) => {
			if v > i64::MAX as f64 || v < i64::MIN as f64 {
				return Err(ftx.error("integer overflow"));
			}
			Value::Int(v as i64)
		},
		Value::Int(v) => Value::Int(v),
		Value::UInt(v) => Value::Int(v.try_into().map_err(|_| ftx.error("integer overflow"))?),
		v => return Err(ftx.error(format!("cannot convert {v:?} to int"))),
	})
}

pub fn optional_none<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	if ftx.this.is_some() || !ftx.args.is_empty() {
		return Err(ftx.error("unsupported function"));
	}
	Ok(OpaqueValue::new(OptionalValue::none()).into())
}

pub fn optional_of<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
	if ftx.this.is_some() {
		return Err(ftx.error("unsupported function"));
	}
	let value: Value = value.load_value(ftx)?;
	// TODO: avoid as_static
	Ok(OpaqueValue::new(OptionalValue::of(value.as_static())).into())
}

pub fn optional_of_non_zero_value<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	value: Argument,
) -> ResolveResult<'a> {
	if ftx.this.is_some() {
		return Err(ftx.error("unsupported function"));
	}
	let value: Value = value.load_value(ftx)?;
	if value.is_zero() {
		Ok(OpaqueValue::new(OptionalValue::none()).into())
	} else {
		Ok(OpaqueValue::new(OptionalValue::of(value.as_static())).into())
	}
}

pub fn optional_value<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let this: Value = this.load_or_arg_value(ftx)?;
	OptionalValue::try_from(this)?
		.value()
		.cloned()
		.map(|v| v.as_static())
		.ok_or_else(|| ftx.error("optional.none() dereference"))
}

pub fn optional_has_value<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
	let this: Value = this.load_or_arg_value(ftx)?;
	Ok(Value::Bool(
		OptionalValue::try_from(this)?.value().is_some(),
	))
}

pub fn optional_or_optional<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	other: Argument,
) -> ResolveResult<'a> {
	let this: Value = this.load_or_arg_value(ftx)?;
	let other: Value = other.load_value(ftx)?;
	let this_opt: OptionalValue = this.clone().try_into()?;
	match this_opt.value() {
		Some(_) => Ok(this),
		None => {
			let _: OptionalValue = other.clone().try_into()?;
			Ok(other)
		},
	}
}

pub fn optional_or_value<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	other: Argument,
) -> ResolveResult<'a> {
	let this: Value = this.load_or_arg_value(ftx)?;
	let other: Value = other.load_value(ftx)?;
	let this_opt: OptionalValue = this.try_into()?;
	match this_opt.value() {
		Some(v) => Ok(v.clone().as_static()),
		None => Ok(other),
	}
}

/// Returns true if a string starts with another string.
///
/// # Example
/// ```cel
/// "abc".startsWith("a") == true
/// ```
pub fn starts_with<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	prefix: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_or_arg(ftx)?;
	let prefix: StringValue = prefix.load_value(ftx)?;
	Ok(Value::Bool(this.as_ref().starts_with(prefix.as_ref())))
}

/// Returns true if a string ends with another string.
///
/// # Example
/// ```cel
/// "abc".endsWith("c") == true
/// ```
pub fn ends_with<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	suffix: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_or_arg(ftx)?;
	let suffix: StringValue = suffix.load_value(ftx)?;
	Ok(Value::Bool(this.as_ref().ends_with(suffix.as_ref())))
}

/// Returns true if a string matches the regular expression.
///
/// # Example
/// ```cel
/// "abc".matches("^[a-z]*$") == true
/// ```
pub fn matches<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	this: This,
	regex: Argument,
) -> ResolveResult<'a> {
	let this: StringValue = this.load_or_arg(ftx)?;
	let regex: StringValue = regex.load_value(ftx)?;
	match regex::Regex::new(&regex) {
		Ok(re) => Ok(Value::Bool(re.is_match(&this))),
		Err(err) => Err(ftx.error(format!("'{}' not a valid regex:\n{err}", regex.as_ref()))),
	}
}

pub use time::{duration, timestamp};

use crate::common::ast::Expr;

pub mod time {
	use chrono::{Datelike, Timelike};

	use super::{FunctionContext, ResolveResult, Result};
	use crate::magic::{Argument, This};
	use crate::objects::StringValue;
	use crate::{ExecutionError, Value};

	/// Duration parses the provided argument into a [`Value::Duration`] value.
	///
	/// The argument must be string, and must be in the format of a duration. See
	/// the [`parse_duration`] documentation for more information on the supported
	/// formats.
	///
	/// # Examples
	/// - `1h` parses as 1 hour
	/// - `1.5h` parses as 1 hour and 30 minutes
	/// - `1h30m` parses as 1 hour and 30 minutes
	/// - `1h30m1s` parses as 1 hour, 30 minutes, and 1 second
	/// - `1ms` parses as 1 millisecond
	/// - `1.5ms` parses as 1 millisecond and 500 microseconds
	/// - `1ns` parses as 1 nanosecond
	/// - `1.5ns` parses as 1 nanosecond (sub-nanosecond durations not supported)
	pub fn duration<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
		let value: StringValue = value.load_value(ftx)?;
		Ok(Value::Duration(_duration(value.as_ref())?))
	}

	/// Timestamp parses the provided argument into a [`Value::Timestamp`] value.
	/// The
	pub fn timestamp<'a>(ftx: &mut FunctionContext<'a, '_>, value: Argument) -> ResolveResult<'a> {
		let value: StringValue = value.load_value(ftx)?;
		Ok(Value::Timestamp(
			chrono::DateTime::parse_from_rfc3339(value.as_ref()).map_err(|e| ftx.error(e.to_string()))?,
		))
	}

	/// A wrapper around [`parse_duration`] that converts errors into [`ExecutionError`].
	/// and only returns the duration, rather than returning the remaining input.
	fn _duration(i: &str) -> Result<chrono::Duration> {
		let (_, duration) = crate::duration::parse_duration(i)
			.map_err(|e| ExecutionError::function_error("duration", e.to_string()))?;
		Ok(duration)
	}

	fn _timestamp(i: &str) -> Result<chrono::DateTime<chrono::FixedOffset>> {
		chrono::DateTime::parse_from_rfc3339(i)
			.map_err(|e| ExecutionError::function_error("timestamp", e.to_string()))
	}
	//
	pub fn timestamp_year<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: chrono::DateTime<chrono::FixedOffset> = this.load_or_arg(ftx)?;
		Ok(Value::Int(this.year() as i64))
	}

	pub fn timestamp_month<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: chrono::DateTime<chrono::FixedOffset> = this.load_or_arg(ftx)?;
		Ok(Value::Int(this.month0() as i64))
	}

	pub fn timestamp_year_day<'a>(
		ftx: &mut FunctionContext<'a, '_>,
		this: This,
	) -> ResolveResult<'a> {
		let this: chrono::DateTime<chrono::FixedOffset> = this.load_or_arg(ftx)?;
		Ok(Value::Int(this.ordinal0() as i64))
	}

	pub fn timestamp_month_day<'a>(
		ftx: &mut FunctionContext<'a, '_>,
		this: This,
	) -> ResolveResult<'a> {
		let this: chrono::DateTime<chrono::FixedOffset> = this.load_or_arg(ftx)?;
		Ok(Value::Int(this.day0() as i64))
	}

	pub fn timestamp_date<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: chrono::DateTime<chrono::FixedOffset> = this.load_or_arg(ftx)?;
		Ok(Value::Int(this.day() as i64))
	}

	pub fn timestamp_weekday<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: chrono::DateTime<chrono::FixedOffset> = this.load_or_arg(ftx)?;
		Ok(Value::Int(this.weekday().num_days_from_sunday() as i64))
	}

	pub fn get_hours<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: Value = this.load_or_arg_value(ftx)?;
		Ok(match this {
			Value::Timestamp(ts) => (ts.hour() as i32).into(),
			Value::Duration(d) => (d.num_hours() as i32).into(),
			_ => return Err(ftx.error("expected timestamp or duration")),
		})
	}

	pub fn get_minutes<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: Value = this.load_or_arg_value(ftx)?;
		Ok(match this {
			Value::Timestamp(ts) => (ts.minute() as i32).into(),
			Value::Duration(d) => (d.num_minutes() as i32).into(),
			_ => return Err(ftx.error("expected timestamp or duration")),
		})
	}

	pub fn get_seconds<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: Value = this.load_or_arg_value(ftx)?;
		Ok(match this {
			Value::Timestamp(ts) => (ts.second() as i32).into(),
			Value::Duration(d) => (d.num_seconds() as i32).into(),
			_ => return Err(ftx.error("expected timestamp or duration")),
		})
	}

	pub fn get_milliseconds<'a>(ftx: &mut FunctionContext<'a, '_>, this: This) -> ResolveResult<'a> {
		let this: Value = this.load_or_arg_value(ftx)?;
		Ok(match this {
			Value::Timestamp(ts) => (ts.timestamp_subsec_millis() as i32).into(),
			Value::Duration(d) => (d.num_milliseconds() as i32).into(),
			_ => return Err(ftx.error("expected timestamp or duration")),
		})
	}
}

pub fn max<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	// Getting materialized values here is fine; we need to materialize to compare
	let args_len = ftx.args.len();
	if args_len == 0 {
		return Ok(Value::Null);
	}

	if args_len == 1 {
		let value = ftx.value(0)?;
		if let Value::List(values) = value {
			let items = values.as_ref();
			let acc = items.first().unwrap_or(&Value::Null);
			return values
				.as_ref()
				.iter()
				.skip(1)
				.try_fold(acc, |acc, x| match acc.partial_cmp(x) {
					Some(Ordering::Greater) => Ok(acc),
					Some(_) => Ok(x),
					None => Err(ExecutionError::ValuesNotComparable(
						acc.as_static(),
						x.as_static(),
					)),
				})
				.cloned();
		}
		// If there is 1 element, it is obviously the max
		return Ok(value);
	}

	let mut values_a = ftx.value_iter();
	let values_b = ftx.value_iter();
	let acc = values_a.next().transpose()?.unwrap_or(Value::Null);
	values_b.skip(1).try_fold(acc, |acc, x| {
		let x = x?;
		match acc.partial_cmp(&x) {
			Some(Ordering::Greater) => Ok(acc),
			Some(_) => Ok(x),
			None => Err(ExecutionError::ValuesNotComparable(
				acc.as_static(),
				x.as_static(),
			)),
		}
	})
}
//
pub fn min<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	let args_len = ftx.args.len();
	if args_len == 0 {
		return Ok(Value::Null);
	}

	if args_len == 1 {
		let value = ftx.value(0)?;
		if let Value::List(values) = value {
			let items = values.as_ref();
			let acc = items.first().unwrap_or(&Value::Null);
			return values
				.as_ref()
				.iter()
				.skip(1)
				.try_fold(acc, |acc, x| match acc.partial_cmp(x) {
					Some(Ordering::Less) => Ok(acc),
					Some(_) => Ok(x),
					None => Err(ExecutionError::ValuesNotComparable(
						acc.as_static(),
						x.as_static(),
					)),
				})
				.cloned();
		}
		// If there is 1 element, it is obviously the min
		return Ok(value);
	}

	let mut values_a = ftx.value_iter();
	let values_b = ftx.value_iter();
	let acc = values_a.next().transpose()?.unwrap_or(Value::Null);
	values_b.skip(1).try_fold(acc, |acc, x| {
		let x = x?;
		match acc.partial_cmp(&x) {
			Some(Ordering::Less) => Ok(acc),
			Some(_) => Ok(x),
			None => Err(ExecutionError::ValuesNotComparable(
				acc.as_static(),
				x.as_static(),
			)),
		}
	})
}

#[cfg(test)]
mod tests {
	use crate::context::{Context, MapResolver};
	use crate::tests::{test_script, test_script_vars};

	fn assert_script(input: &(&str, &str)) {
		assert_eq!(test_script(input.1, None), Ok(true.into()), "{}", input.0);
	}

	fn assert_error(input: &(&str, &str, &str)) {
		assert_eq!(
			test_script(input.1, None)
				.expect_err("expected error")
				.to_string(),
			input.2,
			"{}",
			input.0
		);
	}

	#[test]
	fn test_size() {
		[
			("size of list", "size([1, 2, 3]) == 3"),
			("size of map", "size({'a': 1, 'b': 2, 'c': 3}) == 3"),
			("size of string", "size('foo') == 3"),
			("size of bytes", "size(b'foo') == 3"),
			("size as a list method", "[1, 2, 3].size() == 3"),
			("size as a string method", "'foobar'.size() == 6"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_has() {
		let tests = vec![
			("map has", "has(foo.bar) == true"),
			("map not has", "has(foo.baz) == false"),
		];

		for (name, script) in tests {
			assert_eq!(
				test_script_vars(
					script,
					&[("foo", std::collections::HashMap::from([("bar", 1)]).into())]
				),
				Ok(true.into()),
				"{name}"
			);
		}
	}

	#[test]
	fn test_map() {
		[
			("map list", "[1, 2, 3].map(x, x * 2) == [2, 4, 6]"),
			("map list 2", "[1, 2, 3].map(y, y + 1) == [2, 3, 4]"),
			(
				"map list filter",
				"[1, 2, 3].map(y, y % 2 == 0, y + 1) == [3]",
			),
			(
				"nested map",
				"[[1, 2], [2, 3]].map(x, x.map(x, x * 2)) == [[2, 4], [4, 6]]",
			),
			(
				"map to list",
				r#"{'John': 'smart'}.map(key, key) == ['John']"#,
			),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_map_with_variable() {
		assert_eq!(
			test_script_vars(
				r#"list.map(c, {"key": c}).size() == 2"#,
				&[("list", vec!["a", "b"].into())]
			),
			Ok(true.into()),
			"map with map literal containing variable"
		);
	}

	#[test]
	fn test_filter() {
		[("filter list", "[1, 2, 3].filter(x, x > 2) == [3]")]
			.iter()
			.for_each(assert_script);
	}

	#[test]
	fn test_all() {
		[
			("all list #1", "[0, 1, 2].all(x, x >= 0)"),
			("all list #2", "[0, 1, 2].all(x, x > 0) == false"),
			("all map", "{0: 0, 1:1, 2:2}.all(x, x >= 0) == true"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_exists() {
		[
			("exist list #1", "[0, 1, 2].exists(x, x > 0)"),
			("exist list #2", "[0, 1, 2].exists(x, x == 3) == false"),
			("exist list #3", "[0, 1, 2, 2].exists(x, x == 2)"),
			("exist map", "{0: 0, 1:1, 2:2}.exists(x, x > 0)"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_exists_one() {
		[
			("exist list #1", "[0, 1, 2].exists_one(x, x > 0) == false"),
			("exist list #2", "[0, 1, 2].exists_one(x, x == 0)"),
			("exist map", "{0: 0, 1:1, 2:2}.exists_one(x, x == 2)"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_max() {
		[
			("max single", "max(1) == 1"),
			("max multiple", "max(1, 2, 3) == 3"),
			("max negative", "max(-1, 0) == 0"),
			("max float", "max(-1.0, 0.0) == 0.0"),
			("max list", "max([1, 2, 3]) == 3"),
			("max empty list", "max([]) == null"),
			("max no args", "max() == null"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_min() {
		[
			("min single", "min(1) == 1"),
			("min multiple", "min(1, 2, 3) == 1"),
			("min negative", "min(-1, 0) == -1"),
			("min float", "min(-1.0, 0.0) == -1.0"),
			(
				"min float multiple",
				"min(1.61803, 3.1415, 2.71828, 1.41421) == 1.41421",
			),
			("min list", "min([1, 2, 3]) == 1"),
			("min empty list", "min([]) == null"),
			("min no args", "min() == null"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_starts_with() {
		[
			("starts with true", "'foobar'.startsWith('foo') == true"),
			("starts with false", "'foobar'.startsWith('bar') == false"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_ends_with() {
		[
			("ends with true", "'foobar'.endsWith('bar') == true"),
			("ends with false", "'foobar'.endsWith('foo') == false"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_timestamp() {
		[
			(
				"comparison",
				"timestamp('2023-05-29T00:00:00Z') > timestamp('2023-05-28T00:00:00Z')",
			),
			(
				"comparison",
				"timestamp('2023-05-29T00:00:00Z') < timestamp('2023-05-30T00:00:00Z')",
			),
			(
				"subtracting duration",
				"timestamp('2023-05-29T00:00:00Z') - duration('24h') == timestamp('2023-05-28T00:00:00Z')",
			),
			(
				"subtracting date",
				"timestamp('2023-05-29T00:00:00Z') - timestamp('2023-05-28T00:00:00Z') == duration('24h')",
			),
			(
				"adding duration",
				"timestamp('2023-05-28T00:00:00Z') + duration('24h') == timestamp('2023-05-29T00:00:00Z')",
			),
			(
				"timestamp string",
				"timestamp('2023-05-28T00:00:00Z').string() == '2023-05-28T00:00:00+00:00'",
			),
			(
				"timestamp getFullYear",
				"timestamp('2023-05-28T00:00:00Z').getFullYear() == 2023",
			),
			(
				"timestamp getMonth",
				"timestamp('2023-05-28T00:00:00Z').getMonth() == 4",
			),
			(
				"timestamp getDayOfMonth",
				"timestamp('2023-05-28T00:00:00Z').getDayOfMonth() == 27",
			),
			(
				"timestamp getDayOfYear",
				"timestamp('2023-05-28T00:00:00Z').getDayOfYear() == 147",
			),
			(
				"timestamp getDate",
				"timestamp('2023-05-28T00:00:00Z').getDate() == 28",
			),
			(
				"timestamp getDayOfWeek",
				"timestamp('2023-05-28T00:00:00Z').getDayOfWeek() == 0",
			),
			(
				"timestamp getHours",
				"timestamp('2023-05-28T02:00:00Z').getHours() == 2",
			),
			(
				"timestamp getMinutes",
				" timestamp('2023-05-28T00:05:00Z').getMinutes() == 5",
			),
			(
				"timestamp getSeconds",
				"timestamp('2023-05-28T00:00:06Z').getSeconds() == 6",
			),
			(
				"timestamp getMilliseconds",
				"timestamp('2023-05-28T00:00:42.123Z').getMilliseconds() == 123",
			),
		]
		.iter()
		.for_each(assert_script);

		[
            (
                "timestamp out of range",
                "timestamp('0000-01-00T00:00:00Z')",
                "Error executing function 'timestamp': input is out of range",
            ),
            (
                "timestamp out of range",
                "timestamp('9999-12-32T23:59:59.999999999Z')",
                "Error executing function 'timestamp': input is out of range",
            ),
            (
                "timestamp overflow",
                "timestamp('9999-12-31T23:59:59Z') + duration('1s')",
                "Overflow from binary operator 'add': Timestamp(9999-12-31T23:59:59+00:00), Duration(TimeDelta { secs: 1, nanos: 0 })",
            ),
            (
                "timestamp underflow",
                "timestamp('0001-01-01T00:00:00Z') - duration('1s')",
                "Overflow from binary operator 'sub': Timestamp(0001-01-01T00:00:00+00:00), Duration(TimeDelta { secs: 1, nanos: 0 })",
            ),
            (
                "timestamp underflow",
                "timestamp('0001-01-01T00:00:00Z') + duration('-1s')",
                "Overflow from binary operator 'add': Timestamp(0001-01-01T00:00:00+00:00), Duration(TimeDelta { secs: -1, nanos: 0 })",
            ),
        ]
        .iter()
        .for_each(assert_error)
	}

	#[test]
	fn test_duration() {
		[
			("duration equal 1", "duration('1s') == duration('1000ms')"),
			("duration equal 2", "duration('1m') == duration('60s')"),
			("duration equal 3", "duration('1h') == duration('60m')"),
			("duration comparison 1", "duration('1m') > duration('1s')"),
			("duration comparison 2", "duration('1m') < duration('1h')"),
			(
				"duration subtraction",
				"duration('1h') - duration('1m') == duration('59m')",
			),
			(
				"duration addition",
				"duration('1h') + duration('1m') == duration('1h1m')",
			),
			("duration getHours", "duration('2h30m45s').getHours() == 2"),
			(
				"duration getMinutes",
				"duration('2h30m45s').getMinutes() == 150",
			),
			(
				"duration getSeconds",
				"duration('2h30m45s').getSeconds() == 9045",
			),
			(
				"duration getMilliseconds",
				"duration('1s500ms').getMilliseconds() == 1500",
			),
			(
				"duration getHours overflow",
				"duration('25h').getHours() == 25",
			),
			(
				"duration getMinutes overflow",
				"duration('90m').getMinutes() == 90",
			),
			(
				"duration getSeconds overflow",
				"duration('90s').getSeconds() == 90",
			),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_timestamp_variable() {
		let ts: chrono::DateTime<chrono::FixedOffset> =
			chrono::DateTime::parse_from_rfc3339("2023-05-29T00:00:00Z").unwrap();
		let mut vars = MapResolver::new();
		vars.add_variable_from_value("ts", crate::Value::Timestamp(ts));

		let program = crate::Program::compile("ts == timestamp('2023-05-29T00:00:00Z')").unwrap();
		let ctx = Context::default();
		let result = program.execute_with(&ctx, &vars).unwrap();
		assert_eq!(result, true.into());
	}

	#[test]
	fn test_chrono_string() {
		[
			("duration", "duration('1h30m').string() == '1h30m0s'"),
			(
				"timestamp",
				"timestamp('2023-05-29T00:00:00Z').string() == '2023-05-29T00:00:00+00:00'",
			),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_contains() {
		let tests = vec![
			("list", "[1, 2, 3].contains(3) == true"),
			("map", "{1: true, 2: true, 3: true}.contains(3) == true"),
			("string", "'foobar'.contains('bar') == true"),
			("bytes", "b'foobar'.contains(b'o') == true"),
		];

		for (name, script) in tests {
			assert_eq!(test_script(script, None), Ok(true.into()), "{name}");
		}
	}

	#[test]
	fn test_matches() {
		let tests = vec![
			("string", "'foobar'.matches('^[a-zA-Z]*$') == true"),
			(
				"map",
				"{'1': 'abc', '2': 'def', '3': 'ghi'}.all(key, key.matches('^[a-zA-Z]*$')) == false",
			),
		];

		for (name, script) in tests {
			assert_eq!(
				test_script(script, None),
				Ok(true.into()),
				".matches failed for '{name}'"
			);
		}
	}

	#[test]
	fn test_matches_err() {
		assert_eq!(
			test_script("'foobar'.matches('(foo') == true", None),
			Err(crate::ExecutionError::FunctionError {
				function: "matches".to_string(),
				message:
					"'(foo' not a valid regex:\nregex parse error:\n    (foo\n    ^\nerror: unclosed group"
						.to_string()
			})
		);
	}

	#[test]
	fn test_string() {
		[
			("string", "'foo'.string() == 'foo'"),
			("int", "10.string() == '10'"),
			("float", "10.5.string() == '10.5'"),
			("bytes", "b'foo'.string() == 'foo'"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_bytes() {
		[
			("string", "bytes('abc') == b'abc'"),
			("bytes", "bytes('abc') == b'\\x61b\\x63'"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_double() {
		[
			("string", "'10'.double() == 10.0"),
			("int", "10.double() == 10.0"),
			("double", "10.0.double() == 10.0"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_uint() {
		[
			("string", "'10'.uint() == 10.uint()"),
			("double", "10.5.uint() == 10.uint()"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn test_int() {
		[
			("string", "'10'.int() == 10"),
			("int", "10.int() == 10"),
			("uint", "10.uint().int() == 10"),
			("double", "10.5.int() == 10"),
		]
		.iter()
		.for_each(assert_script);
	}

	#[test]
	fn no_bool_coercion() {
		[
			("string || bool", "'' || false", "No such overload"),
			("int || bool", "1 || false", "No such overload"),
			("int || bool", "1u || false", "No such overload"),
			("float || bool", "0.1|| false", "No such overload"),
			("list || bool", "[] || false", "No such overload"),
			("map || bool", "{} || false", "No such overload"),
			("null || bool", "null || false", "No such overload"),
		]
		.iter()
		.for_each(assert_error)
	}
}
