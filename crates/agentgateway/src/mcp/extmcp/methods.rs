// Wire-name constants sourced from rmcp so match arms stay coupled to the
// crate-of-truth instead of free-floating string literals. Plain `&str`
// re-exports (not the ZSTs) so they're usable directly in match patterns.

use rmcp::model::{
	CallToolRequestMethod, ConstString, GetPromptRequestMethod, ListPromptsRequestMethod,
	ListResourceTemplatesRequestMethod, ListResourcesRequestMethod, ListToolsRequestMethod,
	ReadResourceRequestMethod,
};

pub const TOOLS_LIST: &str = ListToolsRequestMethod::VALUE;
pub const TOOLS_CALL: &str = CallToolRequestMethod::VALUE;
pub const PROMPTS_LIST: &str = ListPromptsRequestMethod::VALUE;
pub const PROMPTS_GET: &str = GetPromptRequestMethod::VALUE;
pub const RESOURCES_LIST: &str = ListResourcesRequestMethod::VALUE;
pub const RESOURCES_TEMPLATES_LIST: &str = ListResourceTemplatesRequestMethod::VALUE;
pub const RESOURCES_READ: &str = ReadResourceRequestMethod::VALUE;

pub fn is_list(method: &str) -> bool {
	matches!(
		method,
		TOOLS_LIST | PROMPTS_LIST | RESOURCES_LIST | RESOURCES_TEMPLATES_LIST
	)
}
