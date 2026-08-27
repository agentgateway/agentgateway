use rmcp::model::{
	CallToolRequestMethod, CompleteRequestMethod, ConstString, GetPromptRequestMethod,
	ListPromptsRequestMethod, ListResourceTemplatesRequestMethod, ListResourcesRequestMethod,
	ListToolsRequestMethod, ReadResourceRequestMethod, SubscribeRequestMethod,
	UnsubscribeRequestMethod,
};

// Method names for the non-fanout requests that carry a mutable body. The
// fanout (`*/list`, `initialize`, ...) path resolves method names dynamically.
pub const TOOLS_CALL: &str = CallToolRequestMethod::VALUE;
pub const PROMPTS_GET: &str = GetPromptRequestMethod::VALUE;
pub const RESOURCES_READ: &str = ReadResourceRequestMethod::VALUE;

// Method names for the fanout list requests, one per dedicated `Relay::merge_*`
// function. Used to populate `mcp.methodName` for RBAC authorization (see
// `rbac::McpAuthorizationSet::validate`), so a policy can distinguish listing a
// resource from calling/reading it.
pub const TOOLS_LIST: &str = ListToolsRequestMethod::VALUE;
pub const PROMPTS_LIST: &str = ListPromptsRequestMethod::VALUE;
pub const RESOURCES_LIST: &str = ListResourcesRequestMethod::VALUE;
pub const RESOURCES_TEMPLATES_LIST: &str = ListResourceTemplatesRequestMethod::VALUE;

// Single-target methods that don't run the request-phase hook yet; only the
// response phase fires for them.
pub const REQUEST_PHASE_UNSUPPORTED: &[&str] = &[
	SubscribeRequestMethod::VALUE,
	UnsubscribeRequestMethod::VALUE,
	CompleteRequestMethod::VALUE,
];
