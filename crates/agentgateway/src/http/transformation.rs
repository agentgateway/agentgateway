use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use http::{HeaderName, HeaderValue, Request};
use minijinja::value::Object;
use minijinja::{Environment, Value, context};

use crate::cel;
use crate::cel::{Attribute, ContextBuilder, ExpressionContext};

const REQUEST_HEADER_ATTRIBUTE: &str = "request_header";
const RESPONSE_HEADER_ATTRIBUTE: &str = "header";
const BODY_ATTRIBUTE: &str = "body";
const RESPONSE_ATTRIBUTE: &str = "response";
const JWT_ATTRIBUTE: &str = "jwt";
const MCP_ATTRIBUTE: &str = "mcp";

pub struct Transformation {
	env: Environment<'static>,
	templates: Vec<CompiledTemplate>,
	attributes: HashSet<String>,
}

impl Transformation {
	pub fn ctx(&self) -> ContextBuilder {
		ContextBuilder {
			attributes: self
				.attributes
				.iter()
				.filter_map(|a| match a.as_str() {
					REQUEST_HEADER_ATTRIBUTE => Some(cel::REQUEST_ATTRIBUTE),
					RESPONSE_HEADER_ATTRIBUTE => Some(cel::RESPONSE_ATTRIBUTE),
					_ => None,
				})
				.map(|s| s.to_string())
				.collect(),
			context: Default::default(),
		}
	}
}

struct CompiledTemplate {
	name: String,
	// Assumed to be request for now
	header: HeaderName,
}

fn build(transforms: HashMap<HeaderName, String>) -> anyhow::Result<Transformation> {
	let mut env: Environment<'static> = Environment::new();
	env.add_function("request_header", functions::request_header);
	let mut templates = Vec::new();
	let mut attributes = HashSet::new();
	let id = 0;
	for (k, t) in transforms.into_iter() {
		let name = format!("template_{id}");
		env.add_template_owned(name.clone(), t)?;
		let tmpl = env.get_template(&name)?;
		attributes.extend(tmpl.undeclared_variables(false));
		templates.push(CompiledTemplate { name, header: k });
	}
	Ok(Transformation {
		env,
		templates,
		attributes,
	})
}

impl Transformation {
	pub fn apply(&self, req: &mut crate::http::Request, ctx: ContextBuilder) {
		let v = to_value(ctx);
		for t in self.templates.iter() {
			let tmpl = self.env.get_template(&t.name).expect("template must exist");
			let headers = req.headers();
			let res = tmpl.render(context! {
					STATE => v,
			});
			req.headers_mut().insert(
				t.header.clone(),
				HeaderValue::try_from(res.unwrap_or_else(|_| "template render failed".to_string()))
					.unwrap(),
			);
		}
	}
}

impl Object for ExpressionContext {}

fn to_value(ctx: ContextBuilder) -> Value {
	Value::from_object(ctx.context)
	// Value::from_serialize(ctx.context)
}

mod functions {
	use minijinja::{State, Value};

	use crate::cel::ExpressionContext;

	macro_rules! state {
		($s:ident) => {
			let Some(state_value) = $s.lookup("STATE") else {
				return Default::default();
			};
			let Some(state) = state_value.downcast_object_ref::<ExpressionContext>() else {
				return Default::default();
			};
			let $s = state;
		};
	}

	pub fn request_header(state: &State, key: &str) -> String {
		state!(state);
		state
			.request
			.as_ref()
			.and_then(|r| r.headers.get(key))
			.and_then(|s| {
				std::str::from_utf8(s.as_bytes())
					.ok()
					.map(|s| s.to_string())
			})
			.unwrap_or("".to_string())
	}
}

#[cfg(test)]
#[path = "transformation_tests.rs"]
mod tests;
