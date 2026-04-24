use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::cel::{ContextBuilder, Executor};
use crate::proxy::ProxyError;
use crate::proxy::dtrace::{
	self, AuthorizationResult as TraceAuthorizationResult, AuthorizationRuleMode,
	AuthorizationRuleResult,
};
use crate::*;

#[derive(Clone, Debug, Serialize)]
pub struct HTTPAuthorizationSet(RuleSets);

impl HTTPAuthorizationSet {
	pub fn new(rs: RuleSets) -> Self {
		Self(rs)
	}
	pub fn apply(&self, req: &http::Request) -> anyhow::Result<()> {
		tracing::debug!(info=?http::DebugExtensions(req), "Checking HTTP request");
		let exec = cel::Executor::new_request(req);
		let allowed = self.0.validate(&exec);
		if !allowed {
			anyhow::bail!("HTTP authorization denied");
		}
		Ok(())
	}

	pub fn register(&self, cel: &mut ContextBuilder) {
		self.0.register(cel);
	}
}

#[derive(Clone, Debug)]
pub struct NetworkAuthorizationSet(RuleSets);

impl NetworkAuthorizationSet {
	pub fn new(rs: RuleSets) -> Self {
		Self(rs)
	}

	pub fn apply(&self, source: &crate::cel::SourceContext) -> Result<(), ProxyError> {
		let exec = Executor::new_source(source);
		let allowed = self.0.validate(&exec);
		if !allowed {
			Err(ProxyError::AuthorizationFailed)
		} else {
			Ok(())
		}
	}

	pub fn register(&self, cel: &mut ContextBuilder) {
		self.0.register(cel);
	}

	pub fn merge_rule_set(&mut self, rule_set: RuleSet) {
		self.0.0.push(rule_set);
	}
}

#[apply(schema!)]
pub struct RuleSet {
	#[serde(serialize_with = "se_policies", deserialize_with = "de_policies")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
	pub rules: PolicySet,
}

impl RuleSet {
	pub fn register(&self, cel: &mut ContextBuilder) {
		for rule in &self.rules.allow {
			cel.register_expression(rule.as_ref());
		}
		for rule in &self.rules.deny {
			cel.register_expression(rule.as_ref());
		}
		for rule in &self.rules.require {
			cel.register_expression(rule.as_ref());
		}
	}
}

#[derive(Clone, Debug, Default)]
pub struct PolicySet {
	allow: Vec<Arc<cel::Expression>>,
	deny: Vec<Arc<cel::Expression>>,
	require: Vec<Arc<cel::Expression>>,
}

#[derive(Clone, Debug)]
pub enum Policy {
	Allow(Arc<cel::Expression>),
	Deny(Arc<cel::Expression>),
	Require(Arc<cel::Expression>),
}

#[apply(schema!)]
#[serde(untagged)]
enum RuleSerde {
	Object {
		#[serde(flatten)]
		rule: RuleTypeSerde,
	},
	PlainString(String),
}

#[apply(schema!)]
enum RuleTypeSerde {
	Allow(String),
	Deny(String),
	Require(String),
}

impl PolicySet {
	pub fn new(
		allow: Vec<Arc<cel::Expression>>,
		deny: Vec<Arc<cel::Expression>>,
		require: Vec<Arc<cel::Expression>>,
	) -> Self {
		Self {
			allow,
			deny,
			require,
		}
	}
}

pub fn se_policies<S: Serializer>(t: &PolicySet, serializer: S) -> Result<S::Ok, S::Error> {
	let len = usize::from(!t.allow.is_empty())
		+ usize::from(!t.deny.is_empty())
		+ usize::from(!t.require.is_empty());
	let mut m = serializer.serialize_map(Some(len))?;
	if !t.allow.is_empty() {
		m.serialize_entry("allow", &t.allow)?;
	}
	if !t.deny.is_empty() {
		m.serialize_entry("deny", &t.deny)?;
	}
	if !t.require.is_empty() {
		m.serialize_entry("require", &t.require)?;
	}
	m.end()
}

pub fn de_policies<'de: 'a, 'a, D>(deserializer: D) -> Result<PolicySet, D::Error>
where
	D: Deserializer<'de>,
{
	let raw = Vec::<RuleSerde>::deserialize(deserializer)?;
	let mut res = PolicySet {
		allow: vec![],
		deny: vec![],
		require: vec![],
	};
	for r in raw {
		match r {
			RuleSerde::Object {
				rule: RuleTypeSerde::Allow(allow),
			}
			| RuleSerde::PlainString(allow) => res.allow.push(
				cel::Expression::new_strict(&allow)
					.map(Arc::new)
					.map_err(|e| serde::de::Error::custom(e.to_string()))?,
			),
			RuleSerde::Object {
				rule: RuleTypeSerde::Deny(deny),
			} => res.deny.push(
				cel::Expression::new_strict(deny)
					.map(Arc::new)
					.map_err(|e| serde::de::Error::custom(e.to_string()))?,
			),
			RuleSerde::Object {
				rule: RuleTypeSerde::Require(require),
			} => res.require.push(
				cel::Expression::new_strict(require)
					.map(Arc::new)
					.map_err(|e| serde::de::Error::custom(e.to_string()))?,
			),
		};
	}
	Ok(res)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RuleSets(Vec<RuleSet>);

impl From<Vec<RuleSet>> for RuleSets {
	fn from(value: Vec<RuleSet>) -> Self {
		Self(value)
	}
}

impl RuleSets {
	pub fn register(&self, ctx: &mut ContextBuilder) {
		for rule_set in &self.0 {
			rule_set.register(ctx);
		}
	}
	pub fn validate(&self, exec: &Executor) -> bool {
		let rule_sets = &self.0;
		let has_rules = rule_sets.iter().any(|r| r.has_rules());
		// If there are no rule sets, everyone has access
		let allowed = if !has_rules {
			true
		// If there are any DENY, deny
		} else if rule_sets.iter().any(|r| r.denies(exec)) {
			false
		// All REQUIRE policies must match when present.
		} else if rule_sets.iter().any(|r| !r.all_requires_match(exec)) {
			false
		// If there are any ALLOW, allow
		} else if rule_sets.iter().any(|r| r.allows(exec)) {
			true
		} else {
			// If only deny rules exist (no allow rules), default to allow (denylist semantics).
			// If allow rules exist but none matched, default to deny (allowlist semantics).
			!rule_sets.iter().any(|r| r.has_allow_rules())
		};

		dtrace::trace(|trace| {
			// Re-evaluate the rules for diagnostics. Note we do not short-circuit here.
			let mut rules = Vec::new();

			for rule_set in rule_sets {
				for rule in &rule_set.rules.allow {
					rules.push(AuthorizationRuleResult {
						name: rule.original_expression.clone(),
						matched: exec.eval_bool(rule.as_ref()),
						mode: AuthorizationRuleMode::Allow,
					});
				}
				for rule in &rule_set.rules.deny {
					rules.push(AuthorizationRuleResult {
						name: rule.original_expression.clone(),
						matched: exec.eval_bool(rule.as_ref()),
						mode: AuthorizationRuleMode::Deny,
					});
				}
				for rule in &rule_set.rules.require {
					rules.push(AuthorizationRuleResult {
						name: rule.original_expression.clone(),
						matched: exec.eval_bool(rule.as_ref()),
						mode: AuthorizationRuleMode::Require,
					});
				}
			}

			trace.authorization_result(
				rules,
				if allowed {
					TraceAuthorizationResult::Allow
				} else {
					TraceAuthorizationResult::Deny
				},
			)
		});

		allowed
	}

	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}
}

impl RuleSet {
	pub fn new(rules: PolicySet) -> Self {
		Self { rules }
	}

	pub fn has_rules(&self) -> bool {
		!self.rules.allow.is_empty() || !self.rules.deny.is_empty() || !self.rules.require.is_empty()
	}

	pub fn has_allow_rules(&self) -> bool {
		!self.rules.allow.is_empty()
	}
	pub fn has_require_rules(&self) -> bool {
		!self.rules.require.is_empty()
	}
	pub fn denies(&self, exec: &cel::Executor) -> bool {
		if self.rules.deny.is_empty() {
			false
		} else {
			self
				.rules
				.deny
				.iter()
				.any(|rule| exec.eval_bool(rule.as_ref()))
		}
	}

	pub fn allows(&self, exec: &cel::Executor) -> bool {
		if self.rules.allow.is_empty() {
			false
		} else {
			self
				.rules
				.allow
				.iter()
				.any(|rule| exec.eval_bool(rule.as_ref()))
		}
	}

	pub fn all_requires_match(&self, exec: &cel::Executor) -> bool {
		self
			.rules
			.require
			.iter()
			.all(|rule| exec.eval_bool(rule.as_ref()))
	}
}

#[cfg(any(test, feature = "internal_benches"))]
#[path = "authorization_tests.rs"]
mod tests;
