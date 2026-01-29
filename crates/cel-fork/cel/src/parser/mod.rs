#![allow(clippy::module_inception)]
#[allow(clippy::all)]
mod r#gen;

pub mod references;

pub use crate::common::ast::IdedExpr as Expression;

mod macros;
mod parse;
#[allow(non_snake_case)]
mod parser;

pub use parser::*;
pub use references::ExpressionReferences;
