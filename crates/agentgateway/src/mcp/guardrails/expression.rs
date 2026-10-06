use rmcp::model::{ErrorCode, ErrorData};
use serde_json::Value as Json;
use tracing::debug;

use crate::cel;
use crate::mcp::guardrails::client::PERMISSION_DENIED;
use crate::mcp::guardrails::{ExpressionAction, ExpressionProcessor, MCPBody, Outcome};
use crate::mcp::upstream::IncomingRequestContext;

enum Eval {
	Pass,
	Reject(ErrorData),
	Rewrite(Json),
}

pub(super) fn check<
	P: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + Sync,
>(
	c: &ExpressionProcessor,
	method: &str,
	body: &mut MCPBody<'_, P>,
	req_ctx: &IncomingRequestContext,
	response: bool,
) -> Outcome<P> {
	let outcome = evaluate(c, method, body, req_ctx, response);
	if let Some(error) = body.error() {
		return Outcome::Reject(internal(method, format!("convert payload: {error}")));
	}
	match outcome {
		Eval::Pass => Outcome::Pass,
		Eval::Reject(e) => Outcome::Reject(e),
		Eval::Rewrite(v) => {
			if body.parsed().is_none() {
				debug!(
					method,
					"mcpGuardrails: ignoring expression transform on request without body"
				);
				return Outcome::Pass;
			}
			match P::deserialize(&v) {
				Ok(value) => {
					body.wire = None;
					Outcome::Mutated(value)
				},
				Err(e) => Outcome::Reject(internal(method, format!("decode transform: {e}"))),
			}
		},
	}
}

// Evaluation errors fail closed as internal errors, distinct from a configured reject.
fn evaluate<P: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + Sync>(
	c: &ExpressionProcessor,
	method: &str,
	body: &MCPBody<'_, P>,
	req_ctx: &IncomingRequestContext,
	response: bool,
) -> Eval {
	let exec = req_ctx.executor();
	let exec = if response {
		exec.with_mcp_result(Some(body))
	} else {
		exec.with_mcp_params(
			body
				.is_present()
				.then_some(body as &dyn ::cel::types::dynamic::DynamicType),
		)
	};
	if let Some(condition) = &c.condition {
		match exec.eval(condition).map(|v| v.as_bool()) {
			Ok(Ok(true)) => {},
			Ok(Ok(false)) => return Eval::Pass,
			Ok(Err(e)) => return Eval::Reject(internal(method, format!("condition: {e}"))),
			Err(e) => return Eval::Reject(internal(method, format!("condition: {e}"))),
		}
	}
	let transform = match &c.action {
		ExpressionAction::Reject(message) => {
			return Eval::Reject(ErrorData::new(PERMISSION_DENIED, message.clone(), None));
		},
		ExpressionAction::Transform(t) => t,
	};
	match exec.eval(transform) {
		Ok(cel::Value::Null) => Eval::Pass,
		Ok(v) if body.parsed().is_some() && body.value().is_ok_and(|orig| *orig == v) => Eval::Pass,
		Ok(v) => match v.json() {
			Ok(j) => Eval::Rewrite(j),
			Err(e) => Eval::Reject(internal(method, format!("transform: {e}"))),
		},
		Err(e) => Eval::Reject(internal(method, format!("transform: {e}"))),
	}
}

fn internal(method: &str, reason: String) -> ErrorData {
	debug!(method, %reason, "mcpGuardrails: expression processor failed");
	ErrorData::new(
		ErrorCode::INTERNAL_ERROR,
		format!("mcpGuardrails expression: {reason}"),
		None,
	)
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use assert_matches::assert_matches;
	use rmcp::model::{CallToolRequestParams, ServerResult};
	use serde_json::json;

	use super::*;

	fn check<P: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + Sync>(
		c: &ExpressionProcessor,
		method: &str,
		params: Option<&P>,
		ctx: &IncomingRequestContext,
	) -> Outcome<P> {
		super::check(c, method, &mut MCPBody::new(params), ctx, false)
	}

	fn request_context() -> IncomingRequestContext {
		let mut ctx = IncomingRequestContext::empty();
		ctx.extensions_mut().insert(crate::mcp::MCPInfo::default());
		ctx
	}

	fn expr(s: &str) -> Arc<cel::Expression> {
		Arc::new(cel::Expression::new_strict(s).unwrap())
	}

	fn reject(condition: &str, message: &str) -> ExpressionProcessor {
		ExpressionProcessor {
			condition: Some(expr(condition)),
			action: ExpressionAction::Reject(message.to_string()),
		}
	}

	fn transform(t: &str) -> ExpressionProcessor {
		ExpressionProcessor {
			condition: None,
			action: ExpressionAction::Transform(expr(t)),
		}
	}

	fn call(c: &ExpressionProcessor, params: Json) -> (Outcome<CallToolRequestParams>, Json) {
		let original: CallToolRequestParams = serde_json::from_value(params).unwrap();
		let out = check(c, "tools/call", Some(&original), &request_context());
		let body = match &out {
			Outcome::Mutated(p) => serde_json::to_value(p).unwrap(),
			_ => serde_json::to_value(&original).unwrap(),
		};
		(out, body)
	}

	#[test]
	fn example_expressions_mask_and_reject() {
		let config: Json = serde_norway::from_str(include_str!(
			"../../../../../examples/mcp-guardrails/expression/config.yaml"
		))
		.unwrap();
		let guardrails: crate::mcp::guardrails::McpGuardrails =
			serde_json::from_value(config["mcp"]["policies"]["mcpGuardrails"].clone()).unwrap();
		let rule = |index: usize| {
			let crate::mcp::guardrails::ProcessorKind::Expression(rule) =
				&guardrails.processors[index].kind
			else {
				panic!("expected expression processor")
			};
			rule
		};
		assert!(guardrails.processors[0].runs_request("tools/call"));
		assert!(guardrails.processors[1].runs_request("tools/call"));
		assert!(guardrails.processors[2].runs_response("tools/call"));
		assert_matches!(call(rule(0), json!({"name": "echo", "arguments": {"message": "DO_NOT_SEND alice@example.com"}})).0,
			Outcome::Reject(e) if e.code == PERMISSION_DENIED);

		let params = json!({"name": "echo", "arguments": {"message": "alice@example.com bob@example.org demo-secret-abc demo-secret-xyz"}, "_meta": {"progressToken": "123"}});
		assert_matches!(call(rule(0), params.clone()).0, Outcome::Pass);
		let (out, masked) = call(rule(1), params);
		assert_matches!(out, Outcome::Mutated(_));
		assert_eq!(
			masked["arguments"]["message"],
			"[EMAIL] [EMAIL] demo-secret-abc demo-secret-xyz"
		);
		assert_eq!(masked["_meta"]["progressToken"], "123");
		let result: ServerResult = serde_json::from_value(json!({
			"content": [
				{"type": "text", "text": masked["arguments"]["message"]},
				{"type": "image", "data": "AA==", "mimeType": "image/png"}
			],
			"_meta": {"kept": true}
		}))
		.unwrap();
		let mut body = MCPBody::new(Some(&result));
		let Outcome::Mutated(result) =
			super::check(rule(2), "tools/call", &mut body, &request_context(), true)
		else {
			panic!("expected response masking")
		};
		let result = serde_json::to_value(result).unwrap();
		assert_eq!(
			result["content"][0]["text"],
			"[EMAIL] [EMAIL] [SECRET] [SECRET]"
		);
		assert_eq!(
			result["content"][1],
			json!({"type": "image", "data": "AA==", "mimeType": "image/png"})
		);
		assert_eq!(result["_meta"], json!({"kept": true}));
	}

	#[test]
	fn rejects_when_condition_matches() {
		let c = reject(
			r#"mcp.params.name == "drop_table""#,
			"drop_table requires admin",
		);
		let (out, _) = call(&c, json!({"name": "drop_table", "arguments": {}}));
		assert_matches!(out, Outcome::Reject(e) if e.code == PERMISSION_DENIED && e.message == "drop_table requires admin");
		let (out, _) = call(&c, json!({"name": "search", "arguments": {}}));
		assert_matches!(out, Outcome::Pass);
	}

	#[test]
	fn transforms_request_body() {
		let c = transform(
			r#"mcp.params.with(p, p.merge({"arguments": p.arguments.merge({"limit": min(p.arguments.limit, 100)})}))"#,
		);
		let (out, body) = call(
			&c,
			json!({"name": "search", "arguments": {"q": "x", "limit": 5000}}),
		);
		assert_matches!(out, Outcome::Mutated(_));
		assert_eq!(
			body,
			json!({"name": "search", "arguments": {"q": "x", "limit": 100}})
		);
		let (out, _) = call(
			&c,
			json!({"name": "search", "arguments": {"q": "x", "limit": 5}}),
		);
		assert_matches!(out, Outcome::Pass);
	}

	#[test]
	fn transforms_response_body() {
		let c = transform(
			r#"mcp.result.with(r, r.merge({"tools": r.tools.filter(t, !t.name.startsWith("internal_"))}))"#,
		);
		let tool = |name: &str| json!({"name": name, "inputSchema": {"type": "object"}});
		let original: ServerResult = serde_json::from_value(
			json!({"tools": [tool("search"), tool("internal_debug")], "_meta": {"tenant": "acme"}}),
		)
		.unwrap();
		let out = super::check(
			&c,
			"tools/list",
			&mut MCPBody::new(Some(&original)),
			&request_context(),
			true,
		);
		let body = match &out {
			Outcome::Mutated(r) => serde_json::to_value(r).unwrap(),
			_ => panic!("expected mutation"),
		};
		assert_matches!(
			out,
			Outcome::Mutated(ServerResult::ListToolsResult(r))
				if r.tools.len() == 1 && r.tools[0].name == "search"
		);
		assert_eq!(
			body,
			json!({"tools": [tool("search")], "_meta": {"tenant": "acme"}})
		);
	}

	#[test]
	fn request_meta_is_nested_and_transformable() {
		let c = ExpressionProcessor {
			condition: Some(expr(
				"mcp.params._meta.tenant == 'acme' && !has(mcp.result)",
			)),
			action: ExpressionAction::Transform(expr(
				"mcp.params.merge({'_meta': mcp.params._meta.merge({'checked': true})})",
			)),
		};
		let (out, body) = call(&c, json!({"name": "search", "_meta": {"tenant": "acme"}}));
		assert_matches!(out, Outcome::Mutated(_));
		assert_eq!(
			body,
			json!({"name": "search", "_meta": {"tenant": "acme", "checked": true}})
		);
	}

	#[test]
	fn bodyless_requests_expose_meta_but_ignore_mutations() {
		let meta = serde_json::from_value(json!({"tenant": "acme"})).unwrap();
		let mut body = MCPBody::<Json>::new(None).with_meta(&meta);
		let ctx = request_context();
		assert_matches!(super::check(&reject("mcp.params._meta.tenant == 'acme'", "denied"), "tools/list", &mut body, &ctx, false), Outcome::Reject(e) if e.code == PERMISSION_DENIED);
		assert_matches!(
			super::check(
				&transform("{'_meta': {'tenant': 'changed'}}"),
				"tools/list",
				&mut body,
				&ctx,
				false
			),
			Outcome::Pass
		);
		assert!(body.wire().unwrap().is_none());
		assert_eq!(
			body.value().unwrap().json().unwrap(),
			json!({"_meta": {"tenant": "acme"}})
		);
	}

	#[test]
	fn rejects_invalid_transformed_body() {
		// Invalid rewrites leave the original body intact.
		let c = transform(r#"{"arguments": {}}"#);
		let params = json!({"name": "search", "arguments": {"q": "x"}});
		let (out, body) = call(&c, params.clone());
		assert_matches!(out, Outcome::Reject(e) if e.code == ErrorCode::INTERNAL_ERROR);
		assert_eq!(body, params);
	}

	#[test]
	fn eval_error_fails_closed() {
		let c = reject(r#"mcp.params.nmae == "x""#, "denied");
		let (out, _) = call(&c, json!({"name": "search"}));
		assert_matches!(out, Outcome::Reject(e) if e.code == ErrorCode::INTERNAL_ERROR);
	}
	#[derive(Debug, serde::Deserialize)]
	struct CountedPayload {
		#[serde(flatten)]
		data: Json,
		#[serde(skip)]
		conversions: Arc<std::sync::atomic::AtomicUsize>,
	}

	impl serde::Serialize for CountedPayload {
		fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
			self
				.conversions
				.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			self.data.serialize(serializer)
		}
	}

	#[test]
	fn converts_once_across_when_transform_and_repeated_references() {
		use std::sync::atomic::Ordering;
		let parsed = CountedPayload {
			data: json!({"name": "search", "arguments": {"city": "SF"}, "newProtocolField": true}),
			conversions: Default::default(),
		};
		let c = ExpressionProcessor {
			condition: Some(expr(
				"mcp.params.name == mcp.params.name && mcp.params.arguments.city == 'SF' && mcp.params.newProtocolField",
			)),
			action: ExpressionAction::Transform(expr("mcp.params")),
		};
		assert_matches!(
			check(&c, "tools/call", Some(&parsed), &request_context()),
			Outcome::Pass
		);
		assert_eq!(parsed.conversions.load(Ordering::Relaxed), 1);
	}

	#[test]
	fn caches_survive_processors_and_refresh_after_mutation() {
		use std::sync::atomic::Ordering;
		let parsed = CountedPayload {
			data: json!({"name": "search"}),
			conversions: Default::default(),
		};
		let ctx = request_context();
		let mut body = MCPBody::new(Some(&parsed));
		for _ in 0..2 {
			assert_matches!(
				super::check(
					&transform("mcp.params"),
					"tools/call",
					&mut body,
					&ctx,
					false
				),
				Outcome::Pass
			);
			assert_eq!(
				body.wire().unwrap().unwrap().as_ref(),
				br#"{"name":"search"}"#
			);
		}
		// One CEL conversion and one wire serialization across both processors.
		assert_eq!(parsed.conversions.load(Ordering::Relaxed), 2);
		let out = super::check(
			&transform("mcp.params.merge({'name': 'updated'})"),
			"tools/call",
			&mut body,
			&ctx,
			false,
		);
		let Outcome::Mutated(updated) = out else {
			panic!("expected mutation")
		};
		assert!(body.wire.is_none());
		body.replace(updated);
		assert_eq!(
			body.value().unwrap().json().unwrap(),
			json!({"name": "updated"})
		);
		assert_eq!(
			body.wire().unwrap().unwrap().as_ref(),
			br#"{"name":"updated"}"#
		);

		// Remote processors retain the validated replacement bytes alongside parsed data.
		let remote_wire = bytes::Bytes::from_static(br#"{ "name": "remote" }"#);
		let replacement = serde_json::from_slice(&remote_wire).unwrap();
		body.wire = Some(remote_wire.clone());
		body.replace(replacement);
		assert_eq!(
			body.value().unwrap().json().unwrap(),
			json!({"name": "remote"})
		);
		assert_eq!(body.wire().unwrap().unwrap(), &remote_wire);
	}

	#[derive(Debug, serde::Deserialize)]
	struct UnusedPayload;
	impl serde::Serialize for UnusedPayload {
		fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
			Err(serde::ser::Error::custom("unexpected payload conversion"))
		}
	}

	#[test]
	fn has_cannot_hide_conversion_errors() {
		assert_matches!(check(&reject("has(mcp.params.name)", "denied"), "tools/call", Some(&UnusedPayload), &request_context()), Outcome::Reject(e) if e.code == ErrorCode::INTERNAL_ERROR && e.message.contains("unexpected payload conversion"));
	}

	#[test]
	fn metadata_only_and_short_circuited_rules_do_not_access_payload() {
		for c in [
			reject("has(mcp.methodName)", "denied"),
			reject("false && mcp.params.name == 'x'", "denied"),
			transform("null"),
		] {
			assert_matches!(
				check(&c, "tools/call", Some(&UnusedPayload), &request_context()),
				Outcome::Pass
			);
		}
		assert_matches!(
			check::<Json>(
				&transform("{'ignored': true}"),
				"tools/list",
				None,
				&request_context()
			),
			Outcome::Pass
		);
	}
}
