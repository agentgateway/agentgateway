use rmcp::model::{
	CallToolRequestMethod, CancelTaskMethod, CompleteRequestMethod, ConstString,
	GetPromptRequestMethod, GetTaskMethod, ReadResourceRequestMethod, SubscribeRequestMethod,
	UnsubscribeRequestMethod, UpdateTaskMethod,
};

// Method names for the non-fanout requests that carry a mutable body. The
// fanout (`*/list`, `initialize`, ...) path resolves method names dynamically.
pub const TOOLS_CALL: &str = CallToolRequestMethod::VALUE;
pub const PROMPTS_GET: &str = GetPromptRequestMethod::VALUE;
pub const RESOURCES_READ: &str = ReadResourceRequestMethod::VALUE;
pub const RESOURCES_SUBSCRIBE: &str = SubscribeRequestMethod::VALUE;
pub const RESOURCES_UNSUBSCRIBE: &str = UnsubscribeRequestMethod::VALUE;
pub const TASKS_GET: &str = GetTaskMethod::VALUE;
pub const TASKS_UPDATE: &str = UpdateTaskMethod::VALUE;
pub const TASKS_CANCEL: &str = CancelTaskMethod::VALUE;

// Single-target methods that don't run the request-phase hook yet; only the
// response phase fires for them. `completion/complete` targets a prompt or a
// resource through its `ref`, which doesn't fit the single-identity hook.
pub const REQUEST_PHASE_UNSUPPORTED: &[&str] = &[CompleteRequestMethod::VALUE];
