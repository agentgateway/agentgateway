// Portions of this code are heavily inspired from https://github.com/Kuadrant/wasm-shim/
// Under Apache 2.0 license (https://github.com/Kuadrant/wasm-shim/blob/main/LICENSE)

use std::sync::OnceLock;

pub use agent_http::BufferedBody;
pub use agent_policy::{Attributes, Error, Expression};
use cel::Context;
pub use cel::Value;
pub use cel::types::dynamic::DynamicType;
use flagset::FlagSet;
pub use helpers::*;
pub use types::*;

mod custom;
mod helpers;
mod types;

struct RootContext {
	context: Context,
}

static ROOT_CONTEXT: OnceLock<RootContext> = OnceLock::new();

fn context() -> &'static Context {
	&ROOT_CONTEXT
		.get_or_init(|| {
			let mut ctx = Context::default();
			agent_celx::insert_all(&mut ctx);
			RootContext { context: ctx }
		})
		.context
}

pub fn register_custom_functions(definitions: &str) -> Result<(), Error> {
	custom::register(definitions)
}

#[derive(Debug)]
pub struct ContextBuilder {
	// Attributes used during the request phase: before we
	request_attributes: FlagSet<Attributes>,
	response_attributes: FlagSet<Attributes>,
	logging_attributes: FlagSet<Attributes>,
}

impl Default for ContextBuilder {
	fn default() -> Self {
		Self::new()
	}
}

impl ContextBuilder {
	pub fn new() -> Self {
		Self {
			request_attributes: Default::default(),
			response_attributes: Default::default(),
			logging_attributes: Default::default(),
		}
	}
	/// register_expression registers the given expressions attributes as required attributes.
	/// Callers MUST call this for each expression they wish to call with the context if they want correct results.
	pub fn register_expression(&mut self, expression: &Expression) {
		// TODO: different types
		self.request_attributes |= expression.attributes()
	}
	/// register_log_expression registers the given expressions attributes as required attributes.
	/// This should only be used for "log" expressions. Log expressions are ones that run after the complete
	/// request and response (including the body) are complete. I.e. if its not executed during DropOnLog,
	/// its probably not the correct usage.
	/// The benefit of this compared to register_expression is that we can do more optimal processing of
	/// bodies, as we know they will complete before we need them, so we can lazily observe the body instead
	/// of proactively buffering.
	pub fn register_log_expression(&mut self, expression: &Expression) {
		self.logging_attributes |= expression.attributes()
	}
	pub fn register_log_request(&mut self) {
		self.logging_attributes |= Attributes::Request;
	}
	fn any_has(&self, attr: impl Into<FlagSet<Attributes>>) -> bool {
		let x = attr.into();
		self.request_attributes.contains(x)
			|| self.response_attributes.contains(x)
			|| self.logging_attributes.contains(x)
	}
	fn before_log_has(&self, attr: impl Into<FlagSet<Attributes>>) -> bool {
		let x = attr.into();
		self.request_attributes.contains(x) || self.response_attributes.contains(x)
	}
	fn log_only_has(&self, attr: impl Into<FlagSet<Attributes>>) -> bool {
		let x = attr.into();
		self.logging_attributes.contains(x) && !self.before_log_has(x)
	}
	pub fn maybe_snapshot_response(
		&self,
		res: &mut crate::http::Response,
	) -> Option<ResponseSnapshot> {
		if self.any_has(Attributes::Response)
			|| self.any_has(Attributes::Metadata)
			|| self.any_has(Attributes::Proxy)
		{
			Some(types::snapshot_response(res))
		} else {
			None
		}
	}
	pub fn maybe_snapshot_request(
		&self,
		res: &mut crate::http::Request,
		clear: bool,
	) -> Option<RequestSnapshot> {
		if self.any_has(Attributes::Source)
			|| self.any_has(Attributes::Destination)
			|| self.any_has(Attributes::Request)
			|| self.any_has(Attributes::Llm)
			|| self.any_has(Attributes::Proxy)
			|| self.any_has(Attributes::Backend)
			|| self.any_has(Attributes::Jwt)
			|| self.any_has(Attributes::ApiKey)
			|| self.any_has(Attributes::BasicAuth)
			|| self.any_has(Attributes::Extauthz)
			|| self.any_has(Attributes::Extproc)
			|| self.any_has(Attributes::Metadata)
		{
			// TODO: support partial snapshots based on what is requested
			Some(types::snapshot_request(res, clear))
		} else {
			None
		}
	}
	pub async fn maybe_buffer_request_body(&self, req: &mut crate::http::Request) {
		if self.before_log_has(Attributes::RequestBody) {
			if req.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			let Ok(body) = crate::http::inspect_body(req).await else {
				return;
			};
			req.extensions_mut().insert(BufferedBody::from(body));
		} else if self.log_only_has(Attributes::RequestBody) {
			if req.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			if req
				.extensions()
				.get::<crate::http::RecordedBodyHandle>()
				.is_some()
			{
				return;
			}
			let body = std::mem::replace(req.body_mut(), crate::http::Body::empty());
			let limit = crate::http::buffer_limit(req);
			let (body, handle) = crate::http::RecordedBody::new_with_limit(body, limit);
			*req.body_mut() = crate::http::Body::new(body);
			req.extensions_mut().insert(handle);
		}
	}
	pub async fn maybe_buffer_response_body(&self, resp: &mut crate::http::Response) {
		if self.before_log_has(Attributes::ResponseBody) {
			if resp.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			let Ok(body) = crate::http::inspect_response_body(resp).await else {
				return;
			};
			resp.extensions_mut().insert(BufferedBody::from(body));
		} else if self.log_only_has(Attributes::ResponseBody) {
			if resp.extensions().get::<BufferedBody>().is_some() {
				return;
			}
			if resp
				.extensions()
				.get::<crate::http::RecordedBodyHandle>()
				.is_some()
			{
				return;
			}
			let body = std::mem::replace(resp.body_mut(), crate::http::Body::empty());
			let limit = crate::http::response_buffer_limit(resp);
			let (body, handle) = crate::http::RecordedBody::new_with_limit(body, limit);
			*resp.body_mut() = crate::http::Body::new(body);
			resp.extensions_mut().insert(handle);
		}
	}

	pub fn needs_llm(&self) -> bool {
		self.any_has(Attributes::Llm)
	}

	pub fn needs_llm_prompt(&self) -> bool {
		self.any_has(Attributes::LlmPrompt)
	}
	pub fn needs_llm_completion(&self) -> bool {
		self.any_has(Attributes::LlmCompletion)
	}
	pub fn needs_llm_tool_calls(&self) -> bool {
		self.any_has(Attributes::LlmToolCalls)
	}
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(any(test, feature = "internal_benches"))]
#[path = "benches.rs"]
mod benches;
mod query;
