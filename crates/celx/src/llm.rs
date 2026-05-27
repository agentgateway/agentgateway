use cel::{Context, ExecutionError, FunctionContext, ResolveResult, Value};

const ECONOMY_MAX_OUTPUT_TOKENS: usize = 1024;
const BALANCED_MAX_OUTPUT_TOKENS: usize = 4096;

pub fn insert_all(ctx: &mut Context) {
	ctx.add_qualified_function("llm", "costClass", cost_class);
}

fn cost_class<'a>(ftx: &mut FunctionContext<'a, '_>) -> ResolveResult<'a> {
	if !(1..=4).contains(&ftx.args.len()) {
		return Err(ExecutionError::FunctionError {
			function: "llm.costClass".to_owned(),
			message: format!("expects 1 to 4 arguments, got {}", ftx.args.len()),
		});
	}

	let (economy_max, balanced_max, explicit_tier) = match ftx.args.len() {
		1 => (ECONOMY_MAX_OUTPUT_TOKENS, BALANCED_MAX_OUTPUT_TOKENS, None),
		2 => (
			ECONOMY_MAX_OUTPUT_TOKENS,
			BALANCED_MAX_OUTPUT_TOKENS,
			Some(1),
		),
		3 => (
			ftx.value(1)?.as_unsigned()?,
			ftx.value(2)?.as_unsigned()?,
			None,
		),
		4 => (
			ftx.value(1)?.as_unsigned()?,
			ftx.value(2)?.as_unsigned()?,
			Some(3),
		),
		_ => unreachable!("argument count checked above"),
	};

	if economy_max >= balanced_max {
		return Err(ftx.error(format!(
			"llm.costClass economy threshold ({economy_max}) must be less than balanced threshold ({balanced_max})"
		)));
	}

	if let Some(index) = explicit_tier
		&& let Some(tier) = explicit_tier_value(ftx, index)?
	{
		return normalize_tier(ftx, tier.as_ref());
	}

	let max_output_tokens = ftx.value(0)?.as_unsigned()?;
	let tier = if max_output_tokens > balanced_max {
		"premium"
	} else if max_output_tokens > economy_max {
		"balanced"
	} else {
		"economy"
	};
	Ok(tier.into())
}

fn explicit_tier_value<'a>(
	ftx: &mut FunctionContext<'a, '_>,
	index: usize,
) -> Result<Option<cel::objects::StringValue<'a>>, ExecutionError> {
	let requested = ftx.value(index)?;
	match requested {
		Value::Null => Ok(None),
		Value::String(tier) if tier.as_ref().trim().is_empty() => Ok(None),
		Value::String(tier) => Ok(Some(tier)),
		other => Err(ftx.error(format!(
			"llm.costClass explicit tier must be string or null, got {}",
			other.type_of().as_str()
		))),
	}
}

fn normalize_tier<'a>(ftx: &FunctionContext<'a, '_>, tier: &str) -> ResolveResult<'a> {
	let normalized = tier.trim().to_ascii_lowercase();
	match normalized.as_str() {
		"economy" | "balanced" | "premium" => Ok(normalized.into()),
		_ => Err(ftx.error(format!(
			"llm.costClass explicit tier must be one of economy, balanced, or premium, got {tier}"
		))),
	}
}
