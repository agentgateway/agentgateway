use std::net::{IpAddr, Ipv4Addr};

use ::http::Method;
#[cfg(test)]
use assert_matches::assert_matches;
use divan::Bencher;
use serde_json::json;

use super::*;
use crate::cel::{RequestSnapshot, SourceContext};
use crate::http::authorization::PolicySet;
use crate::http::{Body, jwt};
use crate::mcp::{MCPInfo, ResourceId, ResourceType};

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

fn tool_context(target: &str, name: &str) -> MCPInfo {
	MCPInfo::from(&ResourceType::Tool(ResourceId::new(
		target.to_string(),
		name.to_string(),
	)))
}

fn create_require_policy_set(policies: Vec<&str>) -> PolicySet {
	let mut policy_set = PolicySet::default();
	for p in policies.into_iter() {
		policy_set
			.require
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
	let mcp = tool_context("server", "increment");
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
	let mcp = tool_context("server", "increment");
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
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), true);

	let mcp = tool_context("not-server", "increment");
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
	let mcp = tool_context("server", "increment");
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
	let mcp = tool_context("server", "increment");
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
	let mcp = tool_context("server", "increment");
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
	let mcp = tool_context("server", "decrement");
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
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), false);
}

#[test]
fn test_network_authorization_allows_source_cidr() {
	let rule_set = RuleSet::new(create_policy_set(vec![
		r#"cidr("10.0.0.0/8").containsIP(source.address)"#,
	]));
	let network_authz = NetworkAuthorizationSet::new(vec![rule_set].into());
	let source = SourceContext {
		address: IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
		port: 15000,
		raw_address: IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
		raw_port: 15000,
		tls: None,
		unverified_workload: None,
		connect_headers: http::HeaderMap::new(),
	};

	assert_matches!(
		network_authz.apply(&cel::Executor::new_source(&source)),
		Ok(())
	);
}

#[test]
fn test_network_authorization_deny_takes_precedence() {
	let allow = RuleSet::new(create_policy_set(vec![
		r#"cidr("10.0.0.0/8").containsIP(source.address)"#,
	]));
	let deny = RuleSet::new(create_deny_policy_set(vec![
		r#"cidr("10.1.0.0/16").containsIP(source.address)"#,
	]));
	let network_authz = NetworkAuthorizationSet::new(vec![allow, deny].into());
	let source = SourceContext {
		address: IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
		port: 15000,
		raw_address: IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
		raw_port: 15000,
		tls: None,
		unverified_workload: None,
		connect_headers: http::HeaderMap::new(),
	};

	assert_matches!(
		network_authz.apply(&cel::Executor::new_source(&source)),
		Err(_)
	);
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
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	// "decrement" is denied by second policy
	let mcp = tool_context("server", "decrement");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	// "echo" is not denied by either policy, so it should be allowed
	let mcp = tool_context("server", "echo");
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
		vec![],
	);
	let rbac = RuleSet::new(policy_set);
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));

	// "allowed_tool" matches allow rule → allowed
	let mcp = tool_context("server", "allowed_tool");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), true);

	// "denied_tool" matches deny rule → denied (deny takes precedence)
	let mcp = tool_context("server", "denied_tool");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	// "other_tool" matches neither → denied (allowlist semantics when allow rules exist)
	let mcp = tool_context("server", "other_tool");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);
}

#[test]
fn test_rbac_mcp_context_is_identity_only() {
	let req = req(json!({"sub": "1234567890"}));
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);
	let expr = cel::Expression::new_strict(
		r#"mcp.tool.name == "increment" && !has(mcp.tool.arguments) && !has(mcp.tool.result) && !has(mcp.tool.error)"#,
	)
	.unwrap();

	assert!(exec.eval_bool(&expr));
}

#[test]
fn test_require_only_matching_allows() {
	let require_policies = vec![r#"mcp.tool.name == "increment""#];
	let rbac = RuleSet::new(create_require_policy_set(require_policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_require_only_non_matching_denies() {
	let require_policies = vec![r#"mcp.tool.name == "increment""#];
	let rbac = RuleSet::new(create_require_policy_set(require_policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac]);
	rs.register(&mut ctx);

	let req = req(json!({"sub": "1234567890"}));
	let mcp = tool_context("server", "decrement");
	let exec = cel::Executor::new_mcp(req.as_ref(), &mcp);

	assert_matches!(rs.validate(&exec), false);
}

#[test]
fn test_all_require_rule_sets_must_pass() {
	let require_increment = RuleSet::new(create_require_policy_set(vec![
		r#"mcp.tool.name == "increment""#,
	]));
	let require_admin = RuleSet::new(create_require_policy_set(vec![r#"jwt.role == "admin""#]));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![require_increment, require_admin]);
	rs.register(&mut ctx);

	let admin_req = req(json!({"role": "admin"}));
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(admin_req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), true);

	let user_req = req(json!({"role": "user"}));
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(user_req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);
}

#[test]
fn test_require_is_not_sufficient_when_allow_rules_exist() {
	let require_increment = RuleSet::new(create_require_policy_set(vec![
		r#"mcp.tool.name == "increment""#,
	]));
	let allow_admin = RuleSet::new(create_policy_set(vec![r#"jwt.role == "admin""#]));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![require_increment, allow_admin]);
	rs.register(&mut ctx);

	let user_req = req(json!({"role": "user"}));
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(user_req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	let admin_req = req(json!({"role": "admin"}));
	let mcp = tool_context("server", "increment");
	let exec = cel::Executor::new_mcp(admin_req.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), true);
}

#[test]
fn test_require_act_chain_denies_bad_deep_actor_despite_allow() {
	let require_act_chain = RuleSet::new(create_require_policy_set(vec![
		r#"has(jwt.act) && jwt.act.recurse("act").all(a, a.sub in ["agent-a", "agent-b"])"#,
	]));
	let allow_user = RuleSet::new(create_policy_set(vec![r#"jwt.sub == "user""#]));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![require_act_chain, allow_user]);
	rs.register(&mut ctx);
	let mcp = tool_context("server", "increment");

	let allowed = req(json!({
		"sub": "user",
		"act": {"sub": "agent-a", "act": {"sub": "agent-b"}},
	}));
	let exec = cel::Executor::new_mcp(allowed.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), true);

	let bad_deep_actor = req(json!({
		"sub": "user",
		"act": {"sub": "agent-a", "act": {"sub": "agent-b", "act": {"sub": "rogue"}}},
	}));
	let exec = cel::Executor::new_mcp(bad_deep_actor.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	let malformed = req(json!({"sub": "user", "act": "agent-a"}));
	let exec = cel::Executor::new_mcp(malformed.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	let no_delegation = req(json!({"sub": "user"}));
	let exec = cel::Executor::new_mcp(no_delegation.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);

	let empty_actor = req(json!({"sub": "user", "act": {}}));
	let exec = cel::Executor::new_mcp(empty_actor.as_ref(), &mcp);
	assert_matches!(rs.validate(&exec), false);
}

#[tokio::test]
async fn test_authorization_cel_error_metrics() {
	use prometheus_client::encoding::text::encode;
	use prometheus_client::registry::Registry;

	use crate::test_helpers::proxymock::{
		BIND_KEY, base_gateway, read_body_raw, send_request_headers, simple_mock,
	};

	const TRACE_PATH: &str = "/authorization-cel-error-metrics";
	const DENY: &str = "int(request.headers['x-number']) == 1";
	const SKIPPED_ALLOW: &str = "int(request.headers['x-absent']) == 1";
	const METRIC: &str = "agentgateway_authorization_cel_evaluation_errors_total";

	let mock = simple_mock().await;
	let mut bind = base_gateway(&mock);
	let mut registry = Registry::default();
	Arc::make_mut(&mut bind.pi).metrics = Arc::new(metrics::Metrics::new(
		agent_core::metrics::sub_registry(&mut registry),
		Default::default(),
		bind.pi.cfg.histograms,
	));
	bind
		.attach_gateway_policy(json!({
			"authorization": {"rules": [{"deny": DENY}]}
		}))
		.await;
	let io = bind.serve_http(BIND_KEY);

	let samples = || {
		let mut output = String::new();
		encode(&mut output, &registry).unwrap();
		output
			.lines()
			.filter(|line| line.starts_with(METRIC))
			.map(str::to_owned)
			.collect::<Vec<_>>()
	};

	for (value, status) in [("2", 200), ("1", 403)] {
		let response = send_request_headers(
			io.clone(),
			Method::GET,
			"http://lo/",
			&[("x-number", value)],
		)
		.await;
		assert_eq!(response.status(), status);
		read_body_raw(response.into_body()).await;
	}
	assert!(samples().is_empty());

	let mut trace_rx = dtrace::track_expression(Some(
		cel::Expression::new_strict(format!("request.path == '{TRACE_PATH}'")).unwrap(),
	));
	let url = format!("http://lo{TRACE_PATH}");
	let response =
		send_request_headers(io.clone(), Method::GET, &url, &[("x-number", "invalid")]).await;
	assert_eq!(response.status(), 200);
	read_body_raw(response.into_body()).await;
	let deny_evaluations = tokio::time::timeout(std::time::Duration::from_secs(1), async {
		let mut evaluations = 0;
		while let Some(event) = trace_rx.recv().await {
			let event = serde_json::to_value(event).unwrap();
			let message = &event["message"];
			match message["type"].as_str() {
				Some("cel") if message["expr"] == DENY => {
					assert!(message["result"].get("error").is_some());
					evaluations += 1;
				},
				Some("authorizationResult") => return evaluations,
				_ => {},
			}
		}
		panic!("missing authorization trace")
	})
	.await
	.unwrap();
	assert_eq!(deny_evaluations, 2);
	let after_traced_deny = samples();

	bind
		.attach_gateway_policy(json!({
			"authorization": {"rules": [
				{"require": format!("request.path == '{TRACE_PATH}' ? request.headers['x-number'] : true")},
				{"allow": SKIPPED_ALLOW},
				{"allow": "true"},
			]}
		}))
		.await;
	let mut trace_rx = dtrace::track_expression(Some(
		cel::Expression::new_strict(format!("request.path == '{TRACE_PATH}'")).unwrap(),
	));
	let response = send_request_headers(io, Method::GET, &url, &[("x-number", "2")]).await;
	assert_eq!(response.status(), 403);
	read_body_raw(response.into_body()).await;
	tokio::time::timeout(std::time::Duration::from_secs(1), async {
		while let Some(event) = trace_rx.recv().await {
			let event = serde_json::to_value(event).unwrap();
			let message = &event["message"];
			if message["type"] == "cel" && message["expr"] == SKIPPED_ALLOW {
				assert!(message["result"].get("error").is_some());
				return;
			}
		}
		panic!("missing trace for skipped allow")
	})
	.await
	.unwrap();
	bind
		.attach_frontend_policy(json!({
			"networkAuthorization": {"rules": [
				{"allow": "int(string(source.address)) == 1"},
				{"allow": "source.port == 12345"},
			]}
		}))
		.await;
	let io = bind.serve_http(BIND_KEY);
	let response = send_request_headers(io, Method::GET, "http://lo/", &[("x-number", "1")]).await;
	assert_eq!(response.status(), 403);

	assert_eq!(
		after_traced_deny,
		[format!(r#"{METRIC}{{mode="deny",layer="http"}} 1"#)]
	);
	let mut actual = samples();
	actual.sort();
	assert_eq!(
		actual,
		[
			format!(r#"{METRIC}{{mode="allow",layer="network"}} 1"#),
			format!(r#"{METRIC}{{mode="deny",layer="http"}} 1"#),
			format!(r#"{METRIC}{{mode="require",layer="http"}} 1"#),
		]
	);
}

#[divan::bench]
fn bench(b: Bencher) {
	let policies = vec![r#"mcp.tool.name == "increment" && jwt.user.role == "admin""#];
	let rbac = RuleSet::new(create_policy_set(policies));
	let mut ctx = ContextBuilder::new();
	let rs = RuleSets::from(vec![rbac.clone()]);
	rs.register(&mut ctx);
	let req = req(json!({"role": "admin"}));
	let mcp = tool_context("server", "increment");
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

	Some(cel::snapshot_request(&mut req, true))
}
