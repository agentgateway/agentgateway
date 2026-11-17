#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use agent_llm_ir::{RenderSettings, prepare};

#[path = "../benches/support/workload.rs"]
mod workload;

#[test]
fn reasoning_workload_converts_to_responses() {
	let pair = workload::Pair::new(workload::Protocol::Messages, workload::Protocol::Responses);
	let conversation = workload::conversation_workload(
		pair.client,
		pair.model(),
		64 << 10,
		workload::RequestWorkload::ReasoningSettings,
	);
	let converted = prepare(&conversation.body, pair.client.client(), pair.upstream())
		.and_then(|prepared| prepared.render(RenderSettings::ModelDefaults))
		.unwrap();
	let body: serde_json::Value = serde_json::from_str(&converted.body).unwrap();
	assert_eq!(body["reasoning"]["effort"], "medium");
	assert!(body["reasoning"].get("summary").is_none());
	workload::check_request(
		"reasoning-settings",
		conversation.turns,
		converted.body.as_bytes(),
	);
}
