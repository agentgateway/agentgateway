use std::sync::Arc;

use crate::macros::{impl_conversions, impl_handler};
use crate::objects::{BytesValue, ListValue, OpaqueValue, StringValue};
use crate::{ExecutionError, Expression, FunctionContext, ResolveResult, Value};

impl_conversions!(
    i64 => Value::Int,
    u64 => Value::UInt,
    f64 => Value::Float,
    StringValue<'a> => Value::String,
    BytesValue<'a> => Value::Bytes,
    OpaqueValue => Value::Object,
    bool => Value::Bool,
    ListValue<'a> => Value::List,
);

impl_conversions!(
    chrono::Duration => Value::Duration,
    chrono::DateTime<chrono::FixedOffset> => Value::Timestamp,
);

impl From<i32> for Value<'static> {
    fn from(value: i32) -> Self {
        Value::Int(value as i64)
    }
}

impl From<u32> for Value<'static> {
    fn from(value: u32) -> Self {
        Value::UInt(value as u64)
    }
}

impl From<f32> for Value<'static> {
    fn from(value: f32) -> Self {
        Value::Float(value as f64)
    }
}

/// Describes any type that can be converted from a [`Value`] into itself.
/// This is commonly used to convert from [`Value`] into primitive types,
/// e.g. from `Value::Bool(true) -> true`. This trait is auto-implemented
/// for many CEL-primitive types.
pub trait FromValue<'a> {
    fn from_value(value: &Value<'a>) -> Result<Self, ExecutionError>
    where
        Self: Sized;
}

impl<'a> FromValue<'a> for Arc<str> {
    fn from_value(value: &Value<'a>) -> Result<Self, ExecutionError> {
        let Value::String(sv) = value else {
            return Err(ExecutionError::no_such_key("todo"));
        };
        Ok(sv.as_owned())
    }
}

impl<'a> FromValue<'a> for Value<'a> {
    fn from_value(value: &Value<'a>) -> Result<Self, ExecutionError>
    where
        Self: Sized,
    {
        Ok(value.clone().always_materialize_owned())
    }
}

/// Describes any type that can be converted from a [`FunctionContext`] into
/// itself, for example CEL primitives implement this trait to allow them to
/// be used as arguments to functions. This trait is core to the 'magic function
/// parameter' system. Every argument to a function that can be registered to
/// the CEL context must implement this type.
pub(crate) trait FromContext<'a, 'rf> {
    fn from_context(ctx: &mut FunctionContext<'a, 'rf>) -> Self
    where
        Self: Sized;
}

pub struct This;

impl This {
    pub fn load<'a>(self, ftx: &FunctionContext<'a, '_>) -> Result<Value<'a>, ExecutionError> {
        ftx.this()
    }
    pub fn load_value<'a, T: FromValue<'a>>(
        self,
        ftx: &FunctionContext<'a, '_>,
    ) -> Result<T, ExecutionError> {
        ftx.this()
    }
    pub fn load_or_arg_value<'a>(
        self,
        ftx: &FunctionContext<'a, '_>,
    ) -> Result<Value<'a>, ExecutionError> {
        ftx.this_or_arg()
    }
    pub fn load_or_arg<'a, T: FromValue<'a>>(
        self,
        ftx: &FunctionContext<'a, '_>,
    ) -> Result<T, ExecutionError> {
        ftx.this_or_arg()
    }
}

pub struct Argument(usize);
impl Argument {
    pub fn load<'a>(self, ftx: &FunctionContext<'a, '_>) -> Result<Value<'a>, ExecutionError> {
        let index = self.0;
        ftx.arg(index)
    }
    pub fn load_value<'a, T: FromValue<'a>>(
        self,
        ftx: &FunctionContext<'a, '_>,
    ) -> Result<T, ExecutionError> {
        let index = self.0;
        ftx.arg(index)
    }
    pub fn load_expression<'a>(
        self,
        ftx: &FunctionContext<'a, '_>,
    ) -> Result<&'a Expression, ExecutionError> {
        ftx.expr(self.0)
    }
    pub fn load_identifier<'a>(
        self,
        ftx: &FunctionContext<'a, '_>,
    ) -> Result<&'a str, ExecutionError> {
        ftx.ident(self.0)
    }
}

impl<'a, 'rf> FromContext<'a, 'rf> for Argument {
    fn from_context(ctx: &mut FunctionContext<'a, 'rf>) -> Self {
        let idx = ctx.arg_idx;
        ctx.arg_idx += 1;
        Argument(idx)
    }
}

impl<'a, 'rf> FromContext<'a, 'rf> for This {
    fn from_context(_ctx: &mut FunctionContext<'a, 'rf>) -> Self {
        This
    }
}

pub struct WithFunctionContext;

impl_handler!();
impl_handler!(C1);
impl_handler!(C1, C2);
impl_handler!(C1, C2, C3);
impl_handler!(C1, C2, C3, C4);
impl_handler!(C1, C2, C3, C4, C5);
impl_handler!(C1, C2, C3, C4, C5, C6);
impl_handler!(C1, C2, C3, C4, C5, C6, C7);
impl_handler!(C1, C2, C3, C4, C5, C6, C7, C8);
impl_handler!(C1, C2, C3, C4, C5, C6, C7, C8, C9);

// Heavily inspired by https://users.rust-lang.org/t/common-data-type-for-functions-with-different-parameters-e-g-axum-route-handlers/90207/6
// and https://play.rust-lang.org/?version=stable&mode=debug&edition=2021&gist=c6744c27c2358ec1d1196033a0ec11e4

pub type Function = Box<
    dyn for<'a, 'rf, 'b> Fn(&'b mut FunctionContext<'a, 'rf>) -> ResolveResult<'a> + Send + Sync,
>;

pub trait IntoFunction<T> {
    fn into_function(self) -> Function;
}

impl IntoFunction<Function> for Function {
    fn into_function(self) -> Function {
        self
    }
}
