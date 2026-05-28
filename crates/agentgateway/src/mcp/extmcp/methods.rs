use rmcp::model::{
	CallToolRequestMethod, ConstString, GetPromptRequestMethod, ReadResourceRequestMethod,
};

// Method names for the non-fanout requests that carry a mutable body. The
// fanout (`*/list`, `initialize`, ...) path resolves method names dynamically.
pub const TOOLS_CALL: &str = CallToolRequestMethod::VALUE;
pub const PROMPTS_GET: &str = GetPromptRequestMethod::VALUE;
pub const RESOURCES_READ: &str = ReadResourceRequestMethod::VALUE;
