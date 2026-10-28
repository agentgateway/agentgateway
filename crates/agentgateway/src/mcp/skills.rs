//! Skills extension (SEP-2640). rmcp has no typed support, so requests arrive as `CustomRequest`
//! and results as `CustomResult`. Types mirror the TypeScript definitions in
//! https://github.com/modelcontextprotocol/ext-skills/blob/c1aa87e4c381c82b99aa37471d0eae4ab50f9985/specification/stable/skills.mdx

use rmcp::model::{
	CacheScope, Cursor, JsonObject, MetaObject, RequestMetaObject, Resource, ResultType,
};
use serde::{Deserialize, Serialize};

pub(crate) const EXTENSION: &str = "io.modelcontextprotocol/skills";
pub(crate) const LIST: &str = "skills/list";
pub(crate) const GET: &str = "skills/get";
pub(crate) const RESOURCES_DIRECTORY_READ: &str = "resources/directory/read";

/// https://github.com/modelcontextprotocol/ext-skills/blob/c1aa87e4c381c82b99aa37471d0eae4ab50f9985/specification/stable/skills.mdx#L147
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SkillResource {
	pub uri: String,
	pub digest: String,
	pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Dynamic {
	Dynamic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum SkillResources {
	Files(Vec<SkillResource>),
	Dynamic(Dynamic),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SkillFrontmatter {
	pub name: String,
	pub description: String,
	/// Every other frontmatter field, passed through unchanged.
	#[serde(flatten)]
	pub extra: JsonObject,
}

/// https://github.com/modelcontextprotocol/ext-skills/blob/c1aa87e4c381c82b99aa37471d0eae4ab50f9985/specification/stable/skills.mdx#L166
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Skill {
	pub uri: String,
	pub frontmatter: SkillFrontmatter,
	pub resources: SkillResources,
	#[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<MetaObject>,
}

/// https://github.com/modelcontextprotocol/ext-skills/blob/c1aa87e4c381c82b99aa37471d0eae4ab50f9985/specification/stable/skills.mdx#L282
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListSkillsResult {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub result_type: Option<ResultType>,
	#[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<MetaObject>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next_cursor: Option<Cursor>,
	/// The gateway always sets its own cache hints, so the upstream's are ignored rather than
	/// failing the request on values like a negative TTL.
	#[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
	pub ttl_ms: Option<u64>,
	#[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
	pub cache_scope: Option<CacheScope>,
	pub skills: Vec<Skill>,
}

/// https://github.com/modelcontextprotocol/ext-skills/blob/c1aa87e4c381c82b99aa37471d0eae4ab50f9985/specification/stable/skills.mdx#L365
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GetSkillRequestParams {
	#[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<RequestMetaObject>,
	pub uri: String,
}

/// https://github.com/modelcontextprotocol/ext-skills/blob/c1aa87e4c381c82b99aa37471d0eae4ab50f9985/specification/stable/skills.mdx#L525
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReadResourceDirectoryRequestParams {
	#[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<RequestMetaObject>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub cursor: Option<Cursor>,
	pub uri: String,
}

/// https://github.com/modelcontextprotocol/ext-skills/blob/c1aa87e4c381c82b99aa37471d0eae4ab50f9985/specification/stable/skills.mdx#L558
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadResourceDirectoryResult {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub result_type: Option<ResultType>,
	#[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<MetaObject>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next_cursor: Option<Cursor>,
	/// See `ListSkillsResult::ttl_ms`.
	#[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
	pub ttl_ms: Option<u64>,
	#[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
	pub cache_scope: Option<CacheScope>,
	pub resources: Vec<Resource>,
}
