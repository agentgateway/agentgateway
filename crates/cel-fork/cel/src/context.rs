use std::collections::{BTreeMap, BTreeSet, HashMap};

use hashbrown::Equivalent;

use crate::common::ast::OptimizedExpr;
use crate::functions;
use crate::magic::{Function, IntoFunction};
use crate::objects::{KeyRef, MapValue, TryIntoValue, Value};

/// Declares how a registered function consumes a method-style receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverStyle {
	/// Must be called method-style (`x.f(...)`); a bare call fails at runtime.
	Required,
	/// Never reads a receiver; `x.f(...)` silently discards `x`.
	Forbidden,
	/// Callable either way: the implementation falls back from the receiver to
	/// the first argument (`FunctionContext::this_or_arg`).
	Either,
}

/// Static call-shape metadata for a registered function, consumed by
/// `Program::check` to validate call sites without
/// executing them.
///
/// Arity counts match [`CallSignature`](crate::parser::CallSignature): a
/// method-style receiver counts as one argument, so `x.contains(y)` and
/// `contains(x, y)` are both arity 2. For [`ReceiverStyle::Forbidden`]
/// functions the receiver never substitutes for an argument, and the range
/// describes the bare-call form only.
///
/// Metadata describes what the implementation accepts *today*, not what a
/// specification says it should.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionMeta {
	/// Minimum accepted arity. Calls below this fail at runtime.
	pub min_args: usize,
	/// Highest accepted arity, or `None` if variadic.
	pub max_args: Option<usize>,
	pub receiver: ReceiverStyle,
}

impl FunctionMeta {
	/// A function of exactly `arity` arguments, callable both method-style
	/// and bare.
	pub const fn either(arity: usize) -> Self {
		FunctionMeta {
			min_args: arity,
			max_args: Some(arity),
			receiver: ReceiverStyle::Either,
		}
	}

	/// A function of exactly `arity` arguments that never reads a receiver.
	pub const fn global(arity: usize) -> Self {
		FunctionMeta {
			min_args: arity,
			max_args: Some(arity),
			receiver: ReceiverStyle::Forbidden,
		}
	}

	/// A function of exactly `arity` arguments that must be called
	/// method-style. The receiver counts toward the arity.
	pub const fn method(arity: usize) -> Self {
		FunctionMeta {
			min_args: arity,
			max_args: Some(arity),
			receiver: ReceiverStyle::Required,
		}
	}

	/// Accepts up to `max_args`, for functions with optional trailing
	/// arguments. The arity already given becomes the minimum.
	pub const fn up_to(mut self, max_args: usize) -> Self {
		self.max_args = Some(max_args);
		self
	}

	/// Accepts any number of trailing arguments beyond the arity already
	/// given, which becomes the minimum.
	pub const fn variadic(mut self) -> Self {
		self.max_args = None;
		self
	}
}

/// Context is a collection of variables and functions that can be used
/// by the interpreter to resolve expressions.
///
/// The context can be either a parent context, or a child context. A
/// parent context is created by default and contains all of the built-in
/// functions. A child context can be created by calling `.new_inner_scope()`. The
/// child context has it's own variables (which can be added to), but it
/// will also reference the parent context. This allows for variables to
/// be overridden within the child context while still being able to
/// resolve variables in the child's parents. You can have theoretically
/// have an infinite number of child contexts that reference each-other.
///
/// So why is this important? Well some CEL-macros such as the `.map` macro
/// declare intermediate user-specified identifiers that should only be
/// available within the macro, and should not override variables in the
/// parent context. The `.map` macro can create a child context from the parent, add the
/// intermediate identifier to the child context, and then evaluate the
/// map expression.
///
/// Intermediate variable stored in child context
///               ↓
/// [1, 2, 3].map(x, x * 2) == [2, 4, 6]
///                  ↑
/// Only in scope for the duration of the map expression
pub struct Context {
	pub functions: BTreeMap<String, Function>,
	pub qualified_functions: hashbrown::HashMap<(String, String), Function>,
	/// Call-shape metadata for entries in `functions`, kept separate because
	/// [`Function`] is a public type alias with no room for fields. Registered
	/// via the `*_with_meta` methods; functions without metadata are simply not
	/// statically checked.
	function_metadata: BTreeMap<String, FunctionMeta>,
	/// The method names the environment's opaque/dynamic values may intercept
	/// before dispatch falls back to `functions`. See
	/// [`Context::declare_opaque_methods`]. `None` means the surface is
	/// unknown, which suppresses all static checking of method-style calls.
	opaque_methods: Option<BTreeSet<String>>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct QualifiedKeyRef<'a>(&'a str, &'a str);

impl Equivalent<(String, String)> for QualifiedKeyRef<'_> {
	fn equivalent(&self, key: &(String, String)) -> bool {
		self == &QualifiedKeyRef(&key.0, &key.1)
	}
}

impl Context {
	pub(crate) fn get_qualified_function(&self, base: &str, name: &str) -> Option<&Function> {
		self.qualified_functions.get(&QualifiedKeyRef(base, name))
	}
	pub(crate) fn get_function(&self, name: &str) -> Option<&Function> {
		self.functions.get(name)
	}

	pub fn add_function<T: 'static, F>(&mut self, name: &str, value: F)
	where
		F: IntoFunction<T> + 'static + Send + Sync,
	{
		self
			.functions
			.insert(name.to_string(), value.into_function());
	}

	pub fn add_function_direct(&mut self, name: &str, value: Function) {
		self.functions.insert(name.to_string(), value);
	}

	/// Like [`Context::add_function`], but also declares call-shape metadata so
	/// `Program::check` can validate call sites.
	pub fn add_function_with_meta<T: 'static, F>(&mut self, name: &str, meta: FunctionMeta, value: F)
	where
		F: IntoFunction<T> + 'static + Send + Sync,
	{
		self.add_function(name, value);
		self.function_metadata.insert(name.to_string(), meta);
	}

	/// Like [`Context::add_function_direct`], but also declares call-shape
	/// metadata so `Program::check` can validate call
	/// sites.
	pub fn add_function_direct_with_meta(&mut self, name: &str, meta: FunctionMeta, value: Function) {
		self.add_function_direct(name, value);
		self.function_metadata.insert(name.to_string(), meta);
	}

	pub fn function_meta(&self, name: &str) -> Option<FunctionMeta> {
		self.function_metadata.get(name).copied()
	}

	/// Declares method names that this environment's opaque or dynamic values
	/// may intercept at dispatch (before falling back to the global function
	/// table). May be called multiple times; the names accumulate.
	///
	/// Until this is called, the opaque surface is *unknown* and
	/// `Program::check` will not diagnose any
	/// method-style call, since any name might be a valid opaque method.
	/// Calling this — even with an empty iterator — asserts that the declared
	/// names are the *complete* set of opaque methods, enabling checking of
	/// method-style calls. When unsure whether a name belongs in the set,
	/// include it: an extra name only suppresses a diagnostic, while a missing
	/// one rejects a working expression.
	pub fn declare_opaque_methods<I>(&mut self, names: I)
	where
		I: IntoIterator,
		I::Item: Into<String>,
	{
		self
			.opaque_methods
			.get_or_insert_with(Default::default)
			.extend(names.into_iter().map(Into::into));
	}

	/// See [`Context::declare_opaque_methods`].
	pub fn opaque_methods(&self) -> Option<&BTreeSet<String>> {
		self.opaque_methods.as_ref()
	}

	pub fn add_qualified_function<T: 'static, F>(&mut self, base: &str, name: &str, value: F)
	where
		F: IntoFunction<T> + 'static + Send + Sync,
	{
		self
			.qualified_functions
			.insert((base.to_string(), name.to_string()), value.into_function());
	}
}

impl Default for Context {
	fn default() -> Self {
		let mut ctx = Context {
			functions: Default::default(),
			qualified_functions: Default::default(),
			function_metadata: Default::default(),
			opaque_methods: None,
		};

		ctx.add_function_with_meta("contains", FunctionMeta::either(2), functions::contains);
		ctx.add_function_with_meta("size", FunctionMeta::either(1), functions::size);
		// A zero-argument call yields null rather than erroring, and any receiver
		// is dropped; neither is recoverable from the declared shape.
		ctx.add_function_with_meta("max", FunctionMeta::global(0).variadic(), functions::max);
		ctx.add_function_with_meta("min", FunctionMeta::global(0).variadic(), functions::min);
		ctx.add_function_with_meta(
			"startsWith",
			FunctionMeta::either(2),
			functions::starts_with,
		);
		ctx.add_function_with_meta("endsWith", FunctionMeta::either(2), functions::ends_with);
		ctx.add_function_with_meta("string", FunctionMeta::either(1), functions::string);
		ctx.add_function_with_meta("bytes", FunctionMeta::global(1), functions::bytes);
		ctx.add_function_with_meta("double", FunctionMeta::either(1), functions::double);
		ctx.add_function_with_meta("int", FunctionMeta::either(1), functions::int);
		ctx.add_function_with_meta("uint", FunctionMeta::either(1), functions::uint);
		ctx.add_function_with_meta("type", FunctionMeta::either(1), functions::type_);

		ctx.add_qualified_function("optional", "none", functions::optional_none);
		ctx.add_qualified_function("optional", "of", functions::optional_of);
		ctx.add_qualified_function(
			"optional",
			"ofNonZeroValue",
			functions::optional_of_non_zero_value,
		);
		ctx.add_function_with_meta("value", FunctionMeta::either(1), functions::optional_value);
		ctx.add_function_with_meta(
			"hasValue",
			FunctionMeta::either(1),
			functions::optional_has_value,
		);
		ctx.add_function_with_meta(
			"or",
			FunctionMeta::either(2),
			functions::optional_or_optional,
		);
		ctx.add_function_with_meta(
			"orValue",
			FunctionMeta::either(2),
			functions::optional_or_value,
		);

		ctx.add_function_with_meta("matches", FunctionMeta::either(2), functions::matches);

		{
			ctx.add_function_with_meta("duration", FunctionMeta::global(1), functions::duration);
			ctx.add_function_with_meta("timestamp", FunctionMeta::global(1), functions::timestamp);
			let time_getter = FunctionMeta::either(1);
			ctx.add_function_with_meta("getFullYear", time_getter, functions::time::timestamp_year);
			ctx.add_function_with_meta("getMonth", time_getter, functions::time::timestamp_month);
			ctx.add_function_with_meta(
				"getDayOfYear",
				time_getter,
				functions::time::timestamp_year_day,
			);
			ctx.add_function_with_meta(
				"getDayOfMonth",
				time_getter,
				functions::time::timestamp_month_day,
			);
			ctx.add_function_with_meta("getDate", time_getter, functions::time::timestamp_date);
			ctx.add_function_with_meta(
				"getDayOfWeek",
				time_getter,
				functions::time::timestamp_weekday,
			);
			ctx.add_function_with_meta("getHours", time_getter, functions::time::get_hours);
			ctx.add_function_with_meta("getMinutes", time_getter, functions::time::get_minutes);
			ctx.add_function_with_meta("getSeconds", time_getter, functions::time::get_seconds);
			ctx.add_function_with_meta(
				"getMilliseconds",
				time_getter,
				functions::time::get_milliseconds,
			);
		}

		ctx
	}
}

pub trait VariableResolver<'a> {
	fn resolve(&self, expr: &str) -> Option<Value<'a>>;
	fn variables(&self) -> Option<Value<'a>> {
		None
	}
	fn resolve_member(&self, _expr: &str, _member: &str) -> Option<Value<'a>> {
		None
	}
	fn resolve_direct(&self, _field: &OptimizedExpr) -> Option<Option<Value<'a>>> {
		None
	}
}

pub struct DefaultVariableResolver;

impl<'a> VariableResolver<'a> for DefaultVariableResolver {
	fn resolve(&self, _expr: &str) -> Option<Value<'a>> {
		None
	}
}

pub struct SingleVarResolver<'a, 'rf> {
	base: &'rf dyn VariableResolver<'a>,
	name: &'a str,
	val: Value<'a>,
}

impl<'a, 'rf> SingleVarResolver<'a, 'rf> {
	pub fn new(base: &'rf dyn VariableResolver<'a>, name: &'a str, val: Value<'a>) -> Self {
		SingleVarResolver { base, name, val }
	}
}

impl<'a, 'rf> VariableResolver<'a> for SingleVarResolver<'a, 'rf> {
	fn resolve(&self, expr: &str) -> Option<Value<'a>> {
		if expr == self.name {
			Some(self.val.clone())
		} else {
			self.base.resolve(expr)
		}
	}

	fn variables(&self) -> Option<Value<'a>> {
		let mut variables = match self.base.variables() {
			Some(Value::Map(map)) => map.iter().map(|(k, v)| (k, v.clone())).collect(),
			_ => vector_map::VecMap::new(),
		};
		variables.insert(KeyRef::String(self.name.into()), self.val.clone());
		Some(Value::Map(MapValue::Borrow(variables)))
	}
}

pub struct MapResolver<'a> {
	variables: HashMap<&'a str, Value<'a>>,
}

impl<'a> Default for MapResolver<'a> {
	fn default() -> Self {
		Self::new()
	}
}

impl<'a> MapResolver<'a> {
	pub fn new() -> Self {
		MapResolver {
			variables: Default::default(),
		}
	}

	pub fn add_variable<V>(
		&mut self,
		name: &'a str,
		value: V,
	) -> Result<(), <V as TryIntoValue<'a>>::Error>
	where
		V: TryIntoValue<'a>,
	{
		let v = value.try_into_value()?;
		self.variables.insert(name, v);
		Ok(())
	}

	pub fn add_variable_from_value<V>(&mut self, name: &'a str, value: V)
	where
		V: Into<Value<'a>>,
	{
		self.variables.insert(name, value.into());
	}
}

impl<'a> VariableResolver<'a> for MapResolver<'a> {
	fn resolve(&self, expr: &str) -> Option<Value<'a>> {
		self.variables.get(expr).cloned()
	}

	fn variables(&self) -> Option<Value<'a>> {
		let variables = self
			.variables
			.iter()
			.map(|(k, v)| (KeyRef::String((*k).into()), v.clone()))
			.collect();
		Some(Value::Map(MapValue::Borrow(variables)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Guard against `Option<FunctionMeta>` silently spreading.
	#[test]
	fn every_default_function_has_metadata() {
		let ctx = Context::default();
		let missing: Vec<&str> = ctx
			.functions
			.keys()
			.filter(|name| ctx.function_meta(name).is_none())
			.map(String::as_str)
			.collect();
		assert!(
			missing.is_empty(),
			"functions registered without metadata: {missing:?}"
		);
	}

	#[test]
	fn opaque_surface_tri_state() {
		let mut ctx = Context::default();
		assert!(ctx.opaque_methods().is_none());
		ctx.declare_opaque_methods(std::iter::empty::<String>());
		assert_eq!(ctx.opaque_methods().map(BTreeSet::len), Some(0));
		ctx.declare_opaque_methods(["cookie", "unredacted"]);
		ctx.declare_opaque_methods(["masked"]);
		let declared = ctx.opaque_methods().unwrap();
		assert!(declared.contains("cookie"));
		assert!(declared.contains("unredacted"));
		assert!(declared.contains("masked"));
	}
}
