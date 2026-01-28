extern crate core;
extern crate self as cel;

use std::convert::TryFrom;
use std::sync::Arc;

use thiserror::Error;

mod macros;

pub mod common;
pub mod context;
pub mod parser;

pub use common::ast::IdedExpr;
use common::ast::SelectExpr;
pub use context::Context;
pub use functions::FunctionContext;
pub use objects::{ResolveResult, Value};
use parser::{Expression, ExpressionReferences, Parser};
pub use parser::{ParseError, ParseErrors};
pub mod functions;
mod magic;
pub mod objects;

mod duration;

pub use ser::{Duration, Timestamp};

mod ser;
pub use ser::{SerializationError, to_value};

mod json;
mod optimize;
#[cfg(test)]
mod test;
pub mod types;

pub use json::ConvertToJsonError;
use magic::FromContext;
pub use optimize::Optimizer;

// Re-export the DynamicType derive macro
pub use cel_derive::DynamicType;

use crate::context::{DefaultVariableResolver, VariableResolver};

pub mod extractors {
    pub use crate::magic::{Argument, Function, IntoFunction, This};
}

#[derive(Error, Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ExecutionError {
    #[error("Invalid argument count: expected {expected}, got {actual}")]
    InvalidArgumentCount { expected: usize, actual: usize },
    #[error("Invalid argument type: {:?}", .target)]
    UnsupportedTargetType { target: Value<'static> },
    #[error("Method '{method}' not supported on type '{target:?}'")]
    NotSupportedAsMethod {
        method: String,
        target: Value<'static>,
    },
    /// Indicates that the script attempted to use a value as a key in a map,
    /// but the type of the value was not supported as a key.
    #[error("Unable to use value '{0:?}' as a key")]
    UnsupportedKeyType(Value<'static>),
    #[error("Unexpected type: got '{got}', want '{want}'")]
    UnexpectedType {
        got: &'static str,
        want: &'static str,
    },
    /// Indicates that the script attempted to reference a key on a type that
    /// was missing the requested key.
    #[error("No such key: {0}")]
    NoSuchKey(Arc<str>),
    /// Indicates that the script used an existing operator or function with
    /// values of one or more types for which no overload was declared.
    #[error("No such overload")]
    NoSuchOverload,
    /// Indicates that the script attempted to reference an undeclared variable
    /// method, or function.
    #[error("Undeclared reference to '{0}'")]
    UndeclaredReference(Arc<String>),
    /// Indicates that a function expected to be called as a method, or to be
    /// called with at least one parameter.
    #[error("Missing argument or target")]
    MissingArgumentOrTarget,
    /// Indicates that a comparison could not be performed.
    #[error("{0:?} can not be compared to {1:?}")]
    ValuesNotComparable(Value<'static>, Value<'static>),
    /// Indicates that an operator was used on a type that does not support it.
    #[error("Unsupported unary operator '{0}': {1:?}")]
    UnsupportedUnaryOperator(&'static str, Value<'static>),
    /// Indicates that an unsupported binary operator was applied on two values
    /// where it's unsupported, for example list + map.
    #[error("Unsupported binary operator '{0}': {1:?}, {2:?}")]
    UnsupportedBinaryOperator(&'static str, Value<'static>, Value<'static>),
    /// Indicates that an unsupported type was used to index a map
    #[error("Cannot use value as map index: {0:?}")]
    UnsupportedMapIndex(Value<'static>),
    /// Indicates that an unsupported type was used to index a list
    #[error("Cannot use value as list index: {0:?}")]
    UnsupportedListIndex(Value<'static>),
    /// Indicates that an unsupported type was used to index a list
    #[error("Cannot use value {0:?} to index {1:?}")]
    UnsupportedIndex(Value<'static>, Value<'static>),
    /// Indicates that a function call occurred without an [`Expression::Ident`]
    /// as the function identifier.
    #[error("Unsupported function call identifier type: {0:?}")]
    UnsupportedFunctionCallIdentifierType(Expression),
    /// Indicates that a [`Member::Fields`] construction was attempted
    /// which is not yet supported.
    #[error("Unsupported fields construction: {0:?}")]
    UnsupportedFieldsConstruction(SelectExpr),
    /// Indicates that a function had an error during execution.
    #[error("Error executing function '{function}': {message}")]
    FunctionError { function: String, message: String },
    #[error("Division by zero of {0:?}")]
    DivisionByZero(Value<'static>),
    #[error("Remainder by zero of {0:?}")]
    RemainderByZero(Value<'static>),
    #[error("Overflow from binary operator '{0}': {1:?}, {2:?}")]
    Overflow(&'static str, Value<'static>, Value<'static>),
    #[error("Cannot convert {1:?} to {0}")]
    Conversion(&'static str, Value<'static>),
    #[error("Index out of bounds: {0:?}")]
    IndexOutOfBounds(Value<'static>),
}

impl ExecutionError {
    pub fn no_such_key(name: &str) -> Self {
        ExecutionError::NoSuchKey(Arc::from(name))
    }

    pub fn undeclared_reference(name: &str) -> Self {
        ExecutionError::UndeclaredReference(Arc::new(name.to_string()))
    }

    pub fn invalid_argument_count(expected: usize, actual: usize) -> Self {
        ExecutionError::InvalidArgumentCount { expected, actual }
    }

    pub fn function_error<E: ToString>(function: &str, error: E) -> Self {
        ExecutionError::FunctionError {
            function: function.to_string(),
            message: error.to_string(),
        }
    }

    pub fn unsupported_target_type(target: Value<'static>) -> Self {
        ExecutionError::UnsupportedTargetType { target }
    }

    pub fn not_supported_as_method(method: &str, target: Value<'static>) -> Self {
        ExecutionError::NotSupportedAsMethod {
            method: method.to_string(),
            target,
        }
    }

    pub fn unsupported_key_type(value: Value<'static>) -> Self {
        ExecutionError::UnsupportedKeyType(value)
    }

    pub fn missing_argument_or_target() -> Self {
        ExecutionError::MissingArgumentOrTarget
    }
}

#[derive(Debug)]
pub struct Program {
    expression: Expression,
}

impl Program {
    pub fn compile_with_optimizer<T: Optimizer + 'static>(
        source: &str,
        t: T,
    ) -> Result<Program, ParseErrors> {
        Ok(Self::compile(source)?.optimized_with(t))
    }
    pub fn compile(source: &str) -> Result<Program, ParseErrors> {
        Ok(Self::compile_unoptimized(source)?.optimized())
    }
    pub fn compile_unoptimized(source: &str) -> Result<Program, ParseErrors> {
        let parser = Parser::default();
        parser
            .parse(source)
            .map(|expression| Program { expression })
    }

    fn optimized(self) -> Program {
        Program {
            expression: crate::optimize::Optimize::new().optimize(self.expression),
        }
    }
    fn optimized_with<T: Optimizer + 'static>(self, t: T) -> Program {
        Program {
            expression: crate::optimize::Optimize::new_with_optimizer(t).optimize(self.expression),
        }
    }

    pub fn execute_with<'a, 'vars: 'a, 'rf>(
        &'vars self,
        context: &'vars Context,
        vars: &'rf dyn VariableResolver<'vars>,
    ) -> ResolveResult<'a> {
        Value::resolve(&self.expression, context, vars)
    }

    pub fn execute<'a>(&'a self, context: &'a Context) -> ResolveResult<'a> {
        Value::resolve(&self.expression, context, &DefaultVariableResolver)
    }

    /// Returns the variables and functions referenced by the CEL program
    ///
    /// # Example
    /// ```rust
    /// # use cel::Program;
    /// let program = Program::compile("size(foo) > 0").unwrap();
    /// let references = program.references();
    ///
    /// assert!(references.has_function("size"));
    /// assert!(references.has_variable("foo"));
    /// ```
    pub fn references(&self) -> ExpressionReferences<'_> {
        self.expression.references()
    }

    /// Returns the contained expression
    pub fn expression(&self) -> &Expression {
        &self.expression
    }
}

impl TryFrom<&str> for Program {
    type Error = ParseErrors;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Program::compile(value)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::convert::TryInto;

    use crate::context::{Context, MapResolver};
    use crate::objects::{ResolveResult, Value};
    use crate::{ExecutionError, Program};

    /// Tests the provided script and returns the result. An optional context can be provided.
    pub(crate) fn test_script(script: &str, ctx: Option<Context>) -> ResolveResult<'_> {
        let program = match Program::compile(script) {
            Ok(p) => p,
            Err(e) => panic!("{}", e),
        };
        let ctx = ctx.unwrap_or_default();
        program.execute(&ctx).map(|v| v.as_static())
    }
    /// Tests the provided script and returns the result. An optional context can be provided.
    pub(crate) fn test_script_vars(
        script: &str,
        vars: &[(&str, Value<'static>)],
    ) -> ResolveResult<'static> {
        let mut var_resolver = MapResolver::new();
        for (k, v) in vars {
            var_resolver.add_variable_from_value(k, v.clone());
        }
        let program = match Program::compile(script) {
            Ok(p) => p,
            Err(e) => panic!("{}", e),
        };
        let ctx = Context::default();
        let v = Value::resolve(&program.expression, &ctx, &var_resolver)?;
        Ok(v.as_static())
    }

    #[test]
    fn parse() {
        Program::compile("1 + 1").unwrap();
    }

    #[test]
    fn from_str() {
        let input = "1.1";
        let _p: Program = input.try_into().unwrap();
    }

    #[test]
    fn variables() {
        fn assert_output(script: &str, expected: ResolveResult) {
            assert_eq!(
                test_script_vars(
                    script,
                    &[
                        ("foo", HashMap::from([("bar", 1i64)]).into()),
                        ("arr", vec![1i64, 2, 3].into()),
                        ("str", "foobar".to_string().into()),
                    ],
                ),
                expected
            );
        }

        // Test methods
        assert_output("size([1, 2, 3]) == 3", Ok(true.into()));
        assert_output("size([size([42]), 2, 3]) == 3", Ok(true.into()));
        assert_output("size([]) == 3", Ok(false.into()));

        // Test variable attribute traversals
        assert_output("foo.bar == 1", Ok(true.into()));

        // Test that we can index into an array
        assert_output("arr[0] == 1", Ok(true.into()));

        // Test that we can index into a string
        assert_output(
            "str[0]",
            Err(ExecutionError::NoSuchKey("0".to_string().into())),
        );
    }

    #[test]
    fn references() {
        let p = Program::compile("[1, 1].map(x, x * 2)").unwrap();
        assert!(p.references().has_variable("x"));
        assert_eq!(p.references().variables().len(), 1);
    }

    #[test]
    fn test_execution_errors() {
        let tests = vec![
            (
                "no such key",
                "foo.baz.bar == 1",
                ExecutionError::no_such_key("baz"),
            ),
            (
                "undeclared reference",
                "missing == 1",
                ExecutionError::undeclared_reference("missing"),
            ),
            (
                "undeclared method",
                "1.missing()",
                ExecutionError::undeclared_reference("missing"),
            ),
            (
                "undeclared function",
                "missing(1)",
                ExecutionError::undeclared_reference("missing"),
            ),
            (
                "unsupported key type",
                "{null: true}",
                ExecutionError::unsupported_key_type(Value::Null),
            ),
        ];

        for (name, script, error) in tests {
            assert_eq!(
                test_script_vars(script, &[("foo", HashMap::from([("bar", 1)]).into())]),
                error.into(),
                "{name}"
            );
        }
    }
}
