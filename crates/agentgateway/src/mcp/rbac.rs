use serde::{Deserialize, Serialize};

use crate::cel::ContextBuilder;
use crate::http::authorization::{RuleSet, RuleSets};
use crate::http::jwt::Claims;
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

#[derive(Clone, Debug)]
pub struct McpAuthorizationSet(RuleSets);

impl McpAuthorizationSet {
	pub fn new(rs: RuleSets) -> Self {
		Self(rs)
	}
	pub fn validate(&self, res: &ResourceType, cel: &ContextBuilder) -> bool {
		tracing::debug!("Checking RBAC for resource: {:?}", res);
		self.0.validate(|| {
			cel
				.build_with_mcp(Some(res))
				.map(agent_core::bow::OwnedOrBorrowed::Owned)
				.map_err(Into::into)
		})
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

#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Default)]
pub struct Identity {
	pub claims: Option<Claims>,
}

impl agent_core::trcng::Claim for Identity {
	fn get_claim(&self, key: &str) -> Option<&str> {
		self.get_claim(key, ".")
	}
}

impl Identity {
	pub fn new(claims: Option<Claims>) -> Self {
		Self { claims }
	}
	// Attempts to get the claim from the claims map
	// The key should be split by the key_delimiter and then the map should be searched recursively
	// If the key is not found, it returns None
	// If the key is found, it returns the value
	pub fn get_claim(&self, key: &str, key_delimiter: &str) -> Option<&str> {
		match &self.claims {
			Some(claims) => {
				// Split the key by the delimiter to handle nested lookups
				let keys = key.split(key_delimiter).collect::<Vec<&str>>();

				// Start with the root claims map
				let mut current_value = &claims.inner;

				// Navigate through each key level
				let num_keys = keys.len();
				for (index, key_part) in keys.into_iter().enumerate() {
					// Get the value at this level
					let value = current_value.get(key_part)?;

					// If this is the last key part, return the string value
					if index == num_keys - 1 {
						return value.as_str();
					}

					// Otherwise, try to navigate deeper if it's an object
					current_value = value.as_object()?;
				}

				None
			},
			None => None,
		}
	}
}
