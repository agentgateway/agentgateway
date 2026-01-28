use std::collections::{BTreeMap, HashMap};

use hashbrown::Equivalent;

use crate::common::ast::OptimizedExpr;
use crate::functions;
use crate::magic::{Function, IntoFunction};
use crate::objects::{TryIntoValue, Value};

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
        self.functions
            .insert(name.to_string(), value.into_function());
    }

    pub fn add_function_direct(&mut self, name: &str, value: Function) {
        self.functions.insert(name.to_string(), value);
    }

    pub fn add_qualified_function<T: 'static, F>(&mut self, base: &str, name: &str, value: F)
    where
        F: IntoFunction<T> + 'static + Send + Sync,
    {
        self.qualified_functions
            .insert((base.to_string(), name.to_string()), value.into_function());
    }
}

impl Default for Context {
    fn default() -> Self {
        let mut ctx = Context {
            functions: Default::default(),
            qualified_functions: Default::default(),
        };

        ctx.add_function("contains", functions::contains);
        ctx.add_function("size", functions::size);
        ctx.add_function("max", functions::max);
        ctx.add_function("min", functions::min);
        ctx.add_function("startsWith", functions::starts_with);
        ctx.add_function("endsWith", functions::ends_with);
        ctx.add_function("string", functions::string);
        ctx.add_function("bytes", functions::bytes);
        ctx.add_function("double", functions::double);
        ctx.add_function("int", functions::int);
        ctx.add_function("uint", functions::uint);

        ctx.add_qualified_function("optional", "none", functions::optional_none);
        ctx.add_qualified_function("optional", "of", functions::optional_of);
        ctx.add_qualified_function(
            "optional",
            "ofNonZeroValue",
            functions::optional_of_non_zero_value,
        );
        ctx.add_function("value", functions::optional_value);
        ctx.add_function("hasValue", functions::optional_has_value);
        ctx.add_function("or", functions::optional_or_optional);
        ctx.add_function("orValue", functions::optional_or_value);

        ctx.add_function("matches", functions::matches);

        {
            ctx.add_function("duration", functions::duration);
            ctx.add_function("timestamp", functions::timestamp);
            ctx.add_function("getFullYear", functions::time::timestamp_year);
            ctx.add_function("getMonth", functions::time::timestamp_month);
            ctx.add_function("getDayOfYear", functions::time::timestamp_year_day);
            ctx.add_function("getDayOfMonth", functions::time::timestamp_month_day);
            ctx.add_function("getDate", functions::time::timestamp_date);
            ctx.add_function("getDayOfWeek", functions::time::timestamp_weekday);
            ctx.add_function("getHours", functions::time::get_hours);
            ctx.add_function("getMinutes", functions::time::get_minutes);
            ctx.add_function("getSeconds", functions::time::get_seconds);
            ctx.add_function("getMilliseconds", functions::time::get_milliseconds);
        }

        ctx
    }
}

pub trait VariableResolver<'a> {
    fn resolve(&self, expr: &str) -> Option<Value<'a>>;
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
}
