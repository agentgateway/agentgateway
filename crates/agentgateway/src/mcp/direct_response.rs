use std::sync::Arc;

use rmcp::model::{
	CallToolResult as RmcpCallToolResult, ContentBlock as RmcpContentBlock, RequestId,
	Resource as RmcpResource, Role, ServerJsonRpcMessage, ServerResult,
	TextContent as RmcpTextContent,
};
use serde_json::Value;

use crate::*;

#[apply(schema!)]
pub enum DirectResponse {
	CallTool(CallToolResult),
}

#[apply(schema!)]
pub struct DirectResponseRule {
	pub when: Arc<cel::Expression>,
	pub respond: DirectResponse,
}

/// Policy holding direct-response rules for `tools/call`.
#[apply(schema!)]
pub struct McpDirectResponse {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub rules: Vec<DirectResponseRule>,
}

#[apply(schema!)]
pub struct CallToolResult {
	pub content: Vec<ContentBlock>,
	#[serde(default)]
	pub is_error: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub structured_content: Option<Value>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<serde_json::Map<String, Value>>,
}

#[apply(schema!)]
pub enum ContentBlock {
	Text(TextContent),
	ResourceLink(ResourceLink),
}

#[apply(schema!)]
pub struct TextContent {
	pub text: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub annotations: Option<Annotations>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<serde_json::Map<String, Value>>,
}

#[apply(schema!)]
pub struct ResourceLink {
	pub uri: String,
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub mime_type: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub size: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub annotations: Option<Annotations>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub meta: Option<serde_json::Map<String, Value>>,
}

#[apply(schema!)]
pub struct Annotations {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub audience: Vec<AnnotationRole>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub priority: Option<f32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub last_modified: Option<chrono::DateTime<chrono::Utc>>,
}

#[apply(schema_enum!)]
pub enum AnnotationRole {
	User,
	Assistant,
}

impl DirectResponse {
	pub fn apply(&self, req_id: RequestId) -> ServerJsonRpcMessage {
		let result = match self {
			DirectResponse::CallTool(r) => ServerResult::CallToolResult(r.to_rmcp()),
		};
		ServerJsonRpcMessage::response(result, req_id)
	}
}

impl CallToolResult {
	fn to_rmcp(&self) -> RmcpCallToolResult {
		let content = self.content.iter().map(ContentBlock::to_rmcp).collect();
		let mut r = if self.is_error {
			RmcpCallToolResult::error(content)
		} else {
			RmcpCallToolResult::success(content)
		};
		r.structured_content = self.structured_content.clone();
		r.meta = self.meta.clone().map(rmcp::model::MetaObject);
		r
	}
}

impl ContentBlock {
	fn to_rmcp(&self) -> RmcpContentBlock {
		match self {
			ContentBlock::Text(t) => {
				let mut tc = RmcpTextContent::new(t.text.clone());
				tc.meta = t.meta.clone().map(rmcp::model::MetaObject);
				tc.annotations = t.annotations.as_ref().map(Annotations::to_rmcp);
				RmcpContentBlock::Text(tc)
			},
			ContentBlock::ResourceLink(r) => RmcpContentBlock::ResourceLink(r.to_rmcp()),
		}
	}
}

impl ResourceLink {
	fn to_rmcp(&self) -> RmcpResource {
		let mut res = RmcpResource::new(self.uri.clone(), self.name.clone());
		res.title = self.title.clone();
		res.description = self.description.clone();
		res.mime_type = self.mime_type.clone();
		res.size = self.size.map(u64::from);
		res.meta = self.meta.clone().map(rmcp::model::MetaObject);
		res.annotations = self.annotations.as_ref().map(Annotations::to_rmcp);
		res
	}
}

impl Annotations {
	fn to_rmcp(&self) -> rmcp::model::Annotations {
		let audience = if self.audience.is_empty() {
			None
		} else {
			Some(
				self
					.audience
					.iter()
					.map(|r| match r {
						AnnotationRole::User => Role::User,
						AnnotationRole::Assistant => Role::Assistant,
					})
					.collect(),
			)
		};
		let mut a = rmcp::model::Annotations::default();
		a.audience = audience;
		a.priority = self.priority;
		a.last_modified = self.last_modified.map(|dt| dt.to_rfc3339());
		a
	}
}
