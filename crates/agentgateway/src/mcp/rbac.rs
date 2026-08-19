use std::sync::Arc;

use ::cel::Value;
use ::cel::objects::{KeyRef, MapValue};
use serde::Serialize;
use vector_map::VecMap;

use crate::cel::ContextBuilder;
use crate::http::authorization::{RuleSet, RuleSets};
use crate::*;

#[apply(schema!)]
pub struct McpAuthorization(
	/// CEL authorization rules for MCP tools, prompts, and resources.
	RuleSet,
);

impl McpAuthorization {
	pub fn new(rule_set: RuleSet) -> Self {
		Self(rule_set)
	}

	pub fn into_inner(self) -> RuleSet {
		self.0
	}
}

/// Cheap clone via Arc; this API treats the request as read-only after construction.
#[derive(Clone)]
pub struct CelExecWrapper(Arc<::http::Request<()>>);

impl CelExecWrapper {
	pub fn new(req: ::http::Request<()>) -> CelExecWrapper {
		CelExecWrapper(Arc::new(req))
	}
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAuthorizationSet(RuleSets);

impl McpAuthorizationSet {
	pub fn new(rs: RuleSets) -> Self {
		Self(rs)
	}

	/// Combine rule sets so both apply; see [`RuleSets::merge`].
	pub fn merge(self, other: Self) -> Self {
		Self(self.0.merge(other.0))
	}

	pub fn validate(&self, res: &ResourceType, cel: &CelExecWrapper) -> bool {
		if !self.0.has_rules() {
			return true;
		}
		tracing::debug!("Checking RBAC for resource: {:?}", res);
		let mcp = crate::mcp::MCPInfo::from(res);
		let exec = crate::cel::Executor::new_mcp_request(cel.0.as_ref(), &mcp);
		self.0.validate(&exec)
	}

	pub fn register(&self, cel: &mut ContextBuilder) {
		self.0.register(cel);
	}
}

// ResourceIdentity extracts the ResourceType and ID from
// a type containing that info, such as a request parameter struct.
pub trait ResourceIdentity {
	const KIND: &'static str;
	fn to_resource_type(&self, service: impl Into<String>) -> ResourceType;
	/// Point the access log at this request's identity.
	fn record(&self, log: &mut crate::mcp::MCPInfo, service: impl Into<String>) {
		log.set_resource_type(&self.to_resource_type(service));
	}
}

macro_rules! resource_identity {
	($($ty:ty => ($kind:literal, $variant:ident, $field:ident),)+) => {$(
		impl ResourceIdentity for $ty {
			const KIND: &'static str = $kind;
			fn to_resource_type(&self, service: impl Into<String>) -> ResourceType {
				ResourceType::$variant(ResourceId::new(service.into(), self.$field.to_string()))
			}
		}
	)+};
}

resource_identity! {
	rmcp::model::GetPromptRequestParams => ("prompt", Prompt, name),
	rmcp::model::ReadResourceRequestParams => ("resource", Resource, uri),
	rmcp::model::SubscribeRequestParams => ("resource", Resource, uri),
	rmcp::model::UnsubscribeRequestParams => ("resource", Resource, uri),
	rmcp::model::GetTaskParams => ("task", Task, task_id),
	rmcp::model::UpdateTaskParams => ("task", Task, task_id),
	rmcp::model::CancelTaskParams => ("task", Task, task_id),
}

// Outside the macro: tool calls also carry arguments, which `set_resource_type`
// preserves as-is, so recording must recapture them.
impl ResourceIdentity for rmcp::model::CallToolRequestParams {
	const KIND: &'static str = "tool";
	fn to_resource_type(&self, service: impl Into<String>) -> ResourceType {
		ResourceType::Tool(ResourceId::new(service.into(), self.name.to_string()))
	}
	fn record(&self, log: &mut crate::mcp::MCPInfo, service: impl Into<String>) {
		log.set_resource_type(&self.to_resource_type(service));
		log.capture_call_arguments(self.arguments.clone());
	}
}

#[apply(schema!)]
#[derive(Eq, PartialEq)]
pub enum ResourceType {
	/// The tool being accessed
	Tool(ResourceId),
	/// The prompt being accessed
	Prompt(ResourceId),
	/// The resource being accessed
	Resource(ResourceId),
	/// The task being accessed (SEP-2663); `name` is the task ID.
	Task(ResourceId),
}

impl ResourceType {
	pub fn id(self) -> ResourceId {
		match self {
			ResourceType::Tool(id)
			| ResourceType::Prompt(id)
			| ResourceType::Resource(id)
			| ResourceType::Task(id) => id,
		}
	}
}

impl cel::DynamicType for ResourceType {
	fn materialize(&self) -> Value<'_> {
		let (n, t) = match self {
			ResourceType::Tool(t) => ("tool", t),
			ResourceType::Prompt(t) => ("prompt", t),
			ResourceType::Resource(t) => ("resource", t),
			ResourceType::Task(t) => ("task", t),
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
			(ResourceType::Task(t), "task") => Some(t.materialize()),
			_ => None,
		}
	}
}

#[apply(schema!)]
#[derive(Eq, PartialEq, ::cel::DynamicType)]
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
	fn test_mcp_authorization_empty_rules_short_circuits() {
		let res = tool_resource("server", "increment");

		let no_rule_sets = McpAuthorizationSet::new(RuleSets::from(vec![]));
		assert!(no_rule_sets.validate(&res, &CelExecWrapper::new(req_without_claims())));

		let empty_rule_set = McpAuthorizationSet::new(RuleSets::from(vec![RuleSet::new(
			PolicySet::new(vec![], vec![], vec![]),
		)]));
		assert!(empty_rule_set.validate(&res, &CelExecWrapper::new(req_without_claims())));
	}

	#[test]
	fn test_backend_policies_merge_composes_mcp_authorization() {
		let with_authz = |authz: McpAuthorizationSet| crate::store::BackendPolicies {
			mcp_authorization: Some(authz),
			..Default::default()
		};
		let deny_all = || {
			McpAuthorizationSet::new(RuleSets::from(vec![RuleSet::new(PolicySet::new(
				vec![],
				vec![Arc::new(cel::Expression::new_strict("true").unwrap())],
				vec![],
			))]))
		};
		let res = tool_resource("server", "increment");
		let cel = CelExecWrapper::new(req_without_claims());

		// Higher-precedence allow does not erase a base deny
		let merged = with_authz(deny_all())
			.merge(with_authz(authorization_set("true")))
			.mcp_authorization
			.unwrap();
		assert!(!merged.validate(&res, &cel));

		// A policy on only one side passes through
		for merged in [
			with_authz(deny_all()).merge(Default::default()),
			crate::store::BackendPolicies::default().merge(with_authz(deny_all())),
		] {
			assert!(!merged.mcp_authorization.unwrap().validate(&res, &cel));
		}
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
