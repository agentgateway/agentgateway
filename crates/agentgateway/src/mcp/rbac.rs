use ::cel::Value;
use ::cel::objects::{KeyRef, MapValue};
use serde::{Deserialize, Serialize};
use vector_map::VecMap;

use crate::cel::{ContextBuilder, RequestSnapshot};
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

pub struct CelExecWrapper(Arc<Option<RequestSnapshot>>);

impl CelExecWrapper {
	pub fn new(snap: Arc<Option<RequestSnapshot>>) -> CelExecWrapper {
		CelExecWrapper(snap)
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
		let exec = crate::cel::Executor::new_mcp(cel.0.as_ref().as_ref(), res);
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
}
