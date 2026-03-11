use super::*;
use crate::cel::RequestSnapshot;
use crate::http::authorization::PolicySet;
use crate::http::{Body, jwt};
use crate::mcp::{ResourceId, ResourceType};
use ::http::Method;
#[cfg(test)]
use assert_matches::assert_matches;
use divan::Bencher;
use serde_json::json;

fn create_policy_set(policies: Vec<&str>) -> PolicySet {
	let mut policy_set = PolicySet::default();
	for p in policies.into_iter() {
		policy_set
			.allow
			.push(Arc::new(cel::Expression::new_strict(p).unwrap()));
	}
	policy_set
}

fn create_deny_policy_set(policies: Vec<&str>) -> PolicySet {
	let mut policy_set = PolicySet::default();
	for p in policies.into_iter() {
		policy_set
			.deny
			.push(Arc::new(cel::Expression::new_strict(p).unwrap()));
	}
	policy_set
}

#[test]
fn test_rbac_reject_exact_match() {
	let policies = vec![r#"mcp.tool.name == "increment" && jwt.user == "admin""#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), false);
}

#[test]
fn test_rbac_check_exact_match() {
	let policies = vec![r#"mcp.tool.name == "increment" && jwt.sub == "1234567890""#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_rbac_target() {
	let policies = vec![r#"mcp.tool.name == "increment" && mcp.tool.target == "server""#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), true);

	let mcp = ResourceType::Tool(ResourceId::new(
		"not-server".to_string(),
		"increment".to_string(),
	));
	let exec_different_target = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec_different_target), false);
}

#[test]
fn test_rbac_check_contains_match() {
	let policies = vec![r#"mcp.tool.name == "increment" && jwt.groups == "admin""#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"groups": "admin"}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_rbac_check_nested_key_match() {
	let policies = vec![r#"mcp.tool.name == "increment" && jwt.user.role == "admin""#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"user": {"role": "admin"}}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_rbac_check_array_contains_match() {
	let policies = vec![r#"mcp.tool.name == "increment" && jwt.roles.contains("admin")"#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"roles": ["user", "admin", "developer"]}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_deny_only_non_matching_allows() {
	// A deny-only policy targeting "increment" should allow other tools
	let deny_policies = vec![r#"mcp.tool.name == "increment""#];
	let rbac = RuleSet::new(create_deny_policy_set(deny_policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"decrement".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	// "decrement" does not match the deny rule, so it should be allowed
	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_deny_only_matching_denies() {
	// A deny-only policy targeting "increment" should deny that tool
	let deny_policies = vec![r#"mcp.tool.name == "increment""#];
	let rbac = RuleSet::new(create_deny_policy_set(deny_policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), false);
}

#[test]
fn test_stacked_deny_policies() {
	// Two deny-only RuleSets: one denies "increment", another denies "decrement"
	// Other tools should still be allowed
	let rbac1 = RuleSet::new(create_deny_policy_set(vec![
		r#"mcp.tool.name == "increment""#,
	]));
	let rbac2 = RuleSet::new(create_deny_policy_set(vec![
		r#"mcp.tool.name == "decrement""#,
	]));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac1, rbac2]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));

	// "increment" is denied by first policy
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	// "decrement" is denied by second policy
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"decrement".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	// "echo" is not denied by either policy, so it should be allowed
	let mcp = ResourceType::Tool(ResourceId::new("server".to_string(), "echo".to_string()));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_mixed_allow_deny_default_deny() {
	// When both allow and deny rules exist, unmatched resources default to deny
	let policy_set = PolicySet::new(
		vec![Arc::new(
			cel::Expression::new_strict(r#"mcp.tool.name == "allowed_tool""#).unwrap(),
		)],
		vec![Arc::new(
			cel::Expression::new_strict(r#"mcp.tool.name == "denied_tool""#).unwrap(),
		)],
	);
	let rbac = RuleSet::new(policy_set);
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));

	// "allowed_tool" matches allow rule → allowed
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"allowed_tool".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), true);

	// "denied_tool" matches deny rule → denied (deny takes precedence)
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"denied_tool".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	// "other_tool" matches neither → denied (allowlist semantics when allow rules exist)
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"other_tool".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);
}

#[divan::bench]
fn bench(b: Bencher) {
	let policies = vec![r#"mcp.tool.name == "increment" && jwt.user.role == "admin""#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);
	let req = req(json!({"role": "admin"}));
	let mcp = ResourceType::Tool(ResourceId::new(
		"server".to_string(),
		"increment".to_string(),
	));
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	b.bench(|| {
		rs.validate(&exec);
	});
}

fn req(claims: serde_json::Value) -> Option<RequestSnapshot> {
	let mut req = ::http::Request::builder()
		.method(Method::POST)
		.uri("http://example.com/mcp")
		.body(Body::empty())
		.unwrap();
	let serde_json::Value::Object(claims) = claims else {
		unreachable!()
	};
	req.extensions_mut().insert(jwt::Claims {
		inner: claims,
		jwt: Default::default(),
	});

	Some(cel::snapshot_request(&mut req))
}
