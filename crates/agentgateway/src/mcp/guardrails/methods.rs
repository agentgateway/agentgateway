use rmcp::model::{
	CallToolRequestMethod, CompleteRequestMethod, ConstString, GetPromptRequestMethod,
	ReadResourceRequestMethod, SubscribeRequestMethod, UnsubscribeRequestMethod,
};

// Method names for the non-fanout requests that carry a mutable body. The
// fanout (`*/list`, `initialize`, ...) path resolves method names dynamically.
pub const TOOLS_CALL: &str = CallToolRequestMethod::VALUE;
pub const PROMPTS_GET: &str = GetPromptRequestMethod::VALUE;
pub const RESOURCES_READ: &str = ReadResourceRequestMethod::VALUE;

// Single-target methods that don't run the request-phase hook yet; only the
// response phase fires for them.
pub const REQUEST_PHASE_UNSUPPORTED: &[&str] = &[
	SubscribeRequestMethod::VALUE,
	UnsubscribeRequestMethod::VALUE,
	CompleteRequestMethod::VALUE,
];
