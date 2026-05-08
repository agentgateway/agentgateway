use ::cel::Value;
use ::cel::objects::{KeyRef, MapValue};
use serde::{Deserialize, Serialize};
use vector_map::VecMap;

use crate::cel::ContextBuilder;
use crate::http::authorization::{RuleSet, RuleSets};
use crate::*;

#[apply(schema!)]
pub struct McpAuthorization(RuleSet);

impl McpAuthorization {
	pub fn new(rule_set: RuleSet) -> Self {
		Self(rule_set)
	}

	pub fn into_inner(self) -> RuleSet {
		self.0
	}
}

pub struct CelExecWrapper(::http::Request<()>);

impl CelExecWrapper {
	pub fn new(req: ::http::Request<()>) -> CelExecWrapper {
		CelExecWrapper(req)
	}
}
#[derive(Clone, Debug)]
pub struct McpAuthorizationSet(RuleSets);

impl McpAuthorizationSet {
	pub fn new(rs: RuleSets) -> Self {
		Self(rs)
	}
	pub fn validate(&self, res: &ResourceType, cel: &CelExecWrapper) -> bool {
		tracing::debug!("Checking RBAC for resource: {:?}", res);
		let mcp = crate::mcp::MCPInfo::from(res);
		let exec = crate::cel::Executor::new_mcp_request(&cel.0, &mcp);
		self.0.validate(&exec)
	}

	pub fn register(&self, cel: &mut ContextBuilder) {
		self.0.register(cel);
	}
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub enum ResourceType {
	/// The tool being accessed
	Tool(ResourceId),
	/// The prompt being accessed
	Prompt(ResourceId),
	/// The resource being accessed
	Resource(ResourceId),
}

impl cel::DynamicType for ResourceType {
	fn materialize(&self) -> Value<'_> {
		let (n, t) = match self {
			ResourceType::Tool(t) => ("tool", t),
			ResourceType::Prompt(t) => ("prompt", t),
			ResourceType::Resource(t) => ("resource", t),
		};
		Value::Map(MapValue::Borrow(VecMap::from_iter([(
			KeyRef::String(n.into()),
			t.materialize(),
		)])))
	}

	fn field(&self, field: &str) -> Option<Value<'_>> {
		match (self, field) {
			(ResourceType::Tool(t), "tool") => Some(t.materialize()),
			(ResourceType::Prompt(t), "prompt") => Some(t.materialize()),
			(ResourceType::Resource(t), "resource") => Some(t.materialize()),
			_ => None,
		}
	}
}

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq, ::cel::DynamicType)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ResourceId {
	#[serde(default)]
	/// The target of the resource
	target: String,
	#[serde(rename = "name", default)]
	/// The name of the resource
	id: String,
}

impl ResourceId {
	pub fn new(target: String, id: String) -> Self {
		Self { target, id }
	}

	pub fn target(&self) -> &str {
		&self.target
	}

	pub fn name(&self) -> &str {
		&self.id
	}
}

#[cfg(test)]
mod tests {
	use std::sync::Arc;

	use serde_json::json;

	use super::*;
	use crate::http::authorization::PolicySet;

	fn tool_resource(target: &str, name: &str) -> ResourceType {
		ResourceType::Tool(ResourceId::new(target.to_string(), name.to_string()))
	}

	fn req_with_claims(claims: serde_json::Value) -> ::http::Request<()> {
		let mut req = ::http::Request::builder()
			.method(::http::Method::POST)
			.uri("http://example.com/mcp")
			.body(())
			.unwrap();
		let serde_json::Value::Object(claims) = claims else {
			panic!("claims must be a JSON object");
		};
		req.extensions_mut().insert(crate::http::jwt::Claims {
			inner: claims,
			jwt: Default::default(),
		});
		req
	}

	fn req_without_claims() -> ::http::Request<()> {
		::http::Request::builder()
			.method(::http::Method::POST)
			.uri("http://example.com/mcp")
			.body(())
			.unwrap()
	}

	fn authorization_set(expr: &str) -> McpAuthorizationSet {
		let policies = PolicySet::new(
			vec![Arc::new(cel::Expression::new_strict(expr).unwrap())],
			vec![],
			vec![],
		);
		McpAuthorizationSet::new(RuleSets::from(vec![RuleSet::new(policies)]))
	}

	#[test]
	fn test_mcp_authorization_jwt_claim_match() {
		let authz = authorization_set(r#"mcp.tool.name == "increment" && jwt.sub == "1234567890""#);
		let req = req_with_claims(json!({ "sub": "1234567890" }));
		let res = tool_resource("server", "increment");

		assert!(authz.validate(&res, &CelExecWrapper::new(req)));
	}

	#[test]
	fn test_mcp_authorization_jwt_nested_claim_mismatch() {
		let authz = authorization_set(r#"mcp.tool.name == "increment" && jwt.user.role == "admin""#);
		let req = req_with_claims(json!({ "user": { "role": "viewer" } }));
		let res = tool_resource("server", "increment");

		assert!(!authz.validate(&res, &CelExecWrapper::new(req)));
	}

	#[test]
	fn test_mcp_authorization_jwt_claim_required_but_missing() {
		let authz = authorization_set(r#"mcp.tool.name == "increment" && jwt.sub == "1234567890""#);
		let req = req_without_claims();
		let res = tool_resource("server", "increment");

		assert!(!authz.validate(&res, &CelExecWrapper::new(req)));
	}
}
