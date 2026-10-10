use std::borrow::Cow;

use cel::common::ast::{Expr, operators};
use cel::common::value::CelVal;

/// Operations applied to a provider model to recover its public name.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ModelTransformation {
	Identity,
	Prefix(String),
	Suffix(String),
	StripPrefix(String),
	StripSuffix(String),
	/// Operations in execution order, from the outermost CEL expression inward.
	Chain(Vec<ModelTransformation>),
}

impl ModelTransformation {
	pub fn apply<'a>(&self, model: impl Into<Cow<'a, str>>) -> Option<Cow<'a, str>> {
		let model = model.into();
		match self {
			Self::Identity => Some(model),
			Self::Prefix(prefix) => Some(Cow::Owned(format!("{prefix}{model}"))),
			Self::Suffix(suffix) => {
				let mut model = model.into_owned();
				model.push_str(suffix);
				Some(Cow::Owned(model))
			},
			Self::StripPrefix(prefix) => match model {
				Cow::Borrowed(model) => model.strip_prefix(prefix.as_str()).map(Cow::Borrowed),
				Cow::Owned(mut model) => model.starts_with(prefix.as_str()).then(|| {
					model.drain(..prefix.len());
					Cow::Owned(model)
				}),
			},
			Self::StripSuffix(suffix) => match model {
				Cow::Borrowed(model) => model.strip_suffix(suffix.as_str()).map(Cow::Borrowed),
				Cow::Owned(mut model) => model.ends_with(suffix.as_str()).then(|| {
					model.truncate(model.len() - suffix.len());
					Cow::Owned(model)
				}),
			},
			Self::Chain(steps) => steps
				.iter()
				.try_fold(model, |model, step| step.apply(model)),
		}
	}
}

/// Compile a supported CEL model transformation into reverse operations during
/// config normalization. Callers must check recovered names against the model pattern.
pub fn reverse_model_transformation(
	expression: &crate::cel::Expression,
) -> Option<ModelTransformation> {
	fn literal(expr: &Expr) -> Option<&str> {
		// CEL compilation turns string literals into Inline values during optimization.
		match expr {
			Expr::Literal(CelVal::String(value)) => Some(value.as_str()),
			Expr::Inline(cel::Value::String(value)) => Some(value.as_ref()),
			_ => None,
		}
	}

	fn reverse(expr: &Expr, steps: &mut Vec<ModelTransformation>) -> Option<()> {
		// Undo the outer operation first, then recurse toward llmRequest.model.
		// "vendor/" + llmRequest.model.stripPrefix("public/") compiles to
		// [StripPrefix("vendor/"), Prefix("public/")].
		match expr {
			// llmRequest.model: the identity transformation and recursion's base case.
			Expr::Select(select)
				if !select.test
					&& select.field == "model"
					&& matches!(&select.operand.expr, Expr::Ident(name) if name == "llmRequest") =>
			{
				Some(())
			},
			Expr::Call(call) => match (
				call.func_name.as_str(),
				call.target.as_deref(),
				call.args.as_slice(),
			) {
				// llmRequest.model.stripPrefix("openai/"): restore "openai/".
				("stripPrefix", Some(target), [affix]) => {
					steps.push(ModelTransformation::Prefix(
						literal(&affix.expr)?.to_owned(),
					));
					reverse(&target.expr, steps)
				},
				// llmRequest.model.stripSuffix("-public"): restore "-public".
				("stripSuffix", Some(target), [affix]) => {
					steps.push(ModelTransformation::Suffix(
						literal(&affix.expr)?.to_owned(),
					));
					reverse(&target.expr, steps)
				},
				// "openai/" + llmRequest.model, or llmRequest.model + "-latest".
				// Reversal removes the added literal; a missing affix means no match.
				(operators::ADD, None, [left, right]) => {
					if let Some(prefix) = literal(&left.expr) {
						steps.push(ModelTransformation::StripPrefix(prefix.to_owned()));
						reverse(&right.expr, steps)
					} else if let Some(suffix) = literal(&right.expr) {
						steps.push(ModelTransformation::StripSuffix(suffix.to_owned()));
						reverse(&left.expr, steps)
					} else {
						None
					}
				},
				// llmRequest["model"]: bracket notation for the same identity transformation.
				(operators::INDEX, None, [object, field])
					if matches!(&object.expr, Expr::Ident(name) if name == "llmRequest")
						&& literal(&field.expr) == Some("model") =>
				{
					Some(())
				},
				_ => None,
			},
			_ => None,
		}
	}

	let mut steps = Vec::new();
	reverse(&expression.ast().expr, &mut steps)?;
	Some(match steps.len() {
		0 => ModelTransformation::Identity,
		1 => steps.pop().unwrap(),
		_ => ModelTransformation::Chain(steps),
	})
}

#[cfg(test)]
mod tests {
	use rstest::rstest;

	use super::*;

	#[rstest]
	#[case::identity("llmRequest.model", "gpt-4o", Some("gpt-4o"))]
	#[case::index("llmRequest['model']", "gpt-4o", Some("gpt-4o"))]
	#[case::strip_prefix(
		"llmRequest.model.stripPrefix('openai/')",
		"gpt-4o",
		Some("openai/gpt-4o")
	)]
	#[case::strip_suffix(
		"llmRequest.model.stripSuffix('-public')",
		"gpt-4o",
		Some("gpt-4o-public")
	)]
	#[case::add_prefix("'openai/' + llmRequest.model", "openai/gpt-4o", Some("gpt-4o"))]
	#[case::add_suffix("llmRequest.model + '-latest'", "gpt-4o-latest", Some("gpt-4o"))]
	#[case::missing_prefix("'openai/' + llmRequest.model", "gpt-4o", None)]
	#[case::missing_suffix("llmRequest.model + '-latest'", "gpt-4o", None)]
	#[case::rename_prefix(
		"'vendor/' + llmRequest.model.stripPrefix('public/')",
		"vendor/gpt-4o",
		Some("public/gpt-4o")
	)]
	#[case::strip_both(
		"llmRequest.model.stripPrefix('openai/').stripSuffix('-public')",
		"gpt-4o",
		Some("openai/gpt-4o-public")
	)]
	#[case::add_both(
		"'openai/' + llmRequest.model + '-latest'",
		"openai/gpt-4o-latest",
		Some("gpt-4o")
	)]
	#[case::empty_affix("llmRequest.model.stripPrefix('') + ''", "gpt-4o", Some("gpt-4o"))]
	#[case::empty_model("'openai/' + llmRequest.model", "openai/", Some(""))]
	#[case::unicode("llmRequest.model.stripPrefix('模型/')", "gpt-4o", Some("模型/gpt-4o"))]
	#[case::escaped_literal(
		r#"llmRequest.model.stripPrefix('team\'s/')"#,
		"gpt-4o",
		Some("team's/gpt-4o")
	)]
	#[case::constant("'gpt-4o'", "gpt-4o", None)]
	#[case::other_field("llmRequest.other", "gpt-4o", None)]
	#[case::other_object("request.model", "gpt-4o", None)]
	#[case::other_index("llmRequest['other']", "gpt-4o", None)]
	#[case::presence("has(llmRequest.model)", "gpt-4o", None)]
	#[case::dynamic_affix(
		"llmRequest.model.stripPrefix(request.headers['x-prefix'])",
		"gpt-4o",
		None
	)]
	#[case::dynamic_add("request.headers['x-prefix'] + llmRequest.model", "gpt-4o", None)]
	#[case::repeated_model("llmRequest.model + llmRequest.model", "gpt-4ogpt-4o", None)]
	#[case::unsupported_call("llmRequest.model.lowerAscii()", "gpt-4o", None)]
	#[case::conditional("llmRequest.model == 'public' ? 'gpt-4o' : 'other'", "gpt-4o", None)]
	fn reverse_transformation(
		#[case] expression: &str,
		#[case] upstream_model: &str,
		#[case] expected: Option<&str>,
	) {
		let expression = crate::cel::Expression::new_strict(expression).unwrap();
		let transformation = reverse_model_transformation(&expression);
		let reversed = transformation
			.as_ref()
			.and_then(|t| t.apply(upstream_model));
		assert_eq!(reversed.as_deref(), expected);

		if let Some(public_model) = reversed {
			let request = serde_json::json!({"model": public_model});
			let executor = crate::cel::Executor::new_llm(None, &request);
			assert_eq!(
				executor.eval(&expression).unwrap().json().unwrap(),
				serde_json::json!(upstream_model),
				"recovered model must transform back to the upstream model"
			);
		}
	}
}
