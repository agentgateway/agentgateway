use std::borrow::Cow;

use agent_core::strng;
use serde_json::{Value, json};

use crate::AIError;
use crate::types::completions::Request;

pub(crate) fn response_format(req: &Request) -> Result<Option<Cow<'_, Value>>, AIError> {
	let standard = req.rest.get("response_format").filter(|v| !v.is_null());
	let Some(alias) = req.rest.get("json_schema").filter(|v| !v.is_null()) else {
		return Ok(standard.map(Cow::Borrowed));
	};
	if !alias.is_object() {
		return Err(invalid("json_schema must be an object"));
	}
	let schema = alias.get("schema").unwrap_or(alias);
	if !schema.is_object() {
		return Err(invalid("json_schema.schema must be an object"));
	}
	let wrapped = alias.get("schema").is_some();
	let format = json!({
		"type": "json_schema",
		"json_schema": {
			"name": if wrapped { alias.get("name").cloned().unwrap_or(json!("response")) } else { json!("response") },
			"strict": if wrapped { alias.get("strict").cloned().unwrap_or(json!(true)) } else { json!(true) },
			"schema": schema,
		}
	});
	if let Some(standard) = standard
		&& standard != &format
	{
		return Err(invalid(
			"json_schema and response_format must agree; supply only one",
		));
	}
	Ok(Some(Cow::Owned(format)))
}

pub(crate) fn invalid(message: &'static str) -> AIError {
	AIError::UnsupportedConversion(strng::new(message))
}
