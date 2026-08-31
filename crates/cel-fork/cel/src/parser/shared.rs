use std::error::Error;
use std::fmt::Display;
use std::sync::Arc;

use crate::common::ast::{Expr, IdedExpr, SourceInfo};

pub struct MacroExprHelper<'a> {
	pub(crate) helper: &'a mut ParserHelper,
	pub(crate) id: u64,
}

impl MacroExprHelper<'_> {
	pub fn next_expr(&mut self, expr: Expr) -> IdedExpr {
		self.helper.next_expr_for(self.id, expr)
	}

	pub(crate) fn pos_for(&self, id: u64) -> Option<(isize, isize)> {
		self.helper.source_info.pos_for(id)
	}
}

#[derive(Debug)]
pub struct ParseErrors {
	pub errors: Vec<ParseError>,
}

impl Display for ParseErrors {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		for (index, error) in self.errors.iter().enumerate() {
			if index != 0 {
				writeln!(f)?;
			}
			write!(f, "{error}")?;
		}
		Ok(())
	}
}

impl Error for ParseErrors {}

#[derive(Debug)]
pub struct ParseError {
	pub source: Option<Box<dyn Error + Send + Sync + 'static>>,
	pub pos: (isize, isize),
	pub msg: String,
	pub expr_id: u64,
	pub source_info: Option<Arc<SourceInfo>>,
}

impl Display for ParseError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"ERROR: <input>:{}:{}: {}",
			self.pos.0, self.pos.1, self.msg
		)?;
		if let Some(info) = &self.source_info {
			if let Some(line) = info.snippet(self.pos.0 - 1) {
				write!(f, "\n| {line}")?;
				write!(f, "\n| {:.>width$}", "^", width = self.pos.1 as usize)?;
			}
		}
		Ok(())
	}
}

impl Error for ParseError {}

pub(crate) struct ParserHelper {
	pub(crate) source_info: SourceInfo,
	pub(crate) next_id: u64,
}

impl Default for ParserHelper {
	fn default() -> Self {
		Self {
			source_info: SourceInfo::default(),
			next_id: 1,
		}
	}
}

impl ParserHelper {
	fn next_id_for(&mut self, id: u64) -> u64 {
		let (start, stop) = self.source_info.offset_for(id).expect("invalid offset");
		let id = self.next_id;
		self.source_info.add_offset(id, start, stop);
		self.next_id += 1;
		id
	}

	fn next_expr_for(&mut self, id: u64, expr: Expr) -> IdedExpr {
		IdedExpr {
			id: self.next_id_for(id),
			expr,
		}
	}
}
