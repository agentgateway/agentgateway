use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::marker::PhantomData;

use ::http::header::InvalidHeaderValue;
use ::http::{HeaderName, HeaderValue, Request, header};
use agent_core::prelude::Strng;
use http_body_util::BodyExt;
use minijinja::value::Object;
use minijinja::{Environment, Value, context};
use serde_with::{SerializeAs, TryFromInto, serde_as};

use crate::cel::{Attribute, ContextBuilder, Executor, Expression, ExpressionContext};
use crate::{cel, *};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalTransformationConfig {
	#[serde(default)]
	pub request: Option<LocalTransform>,
	#[serde(default)]
	pub response: Option<LocalTransform>,
}

#[serde_as]
#[derive(Default, Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct LocalTransform {
	#[serde(default)]
	#[serde_as(as = "serde_with::Map<_, _>")]
	pub add: Vec<(Strng, Strng)>,
	#[serde(default)]
	#[serde_as(as = "serde_with::Map<_, _>")]
	pub set: Vec<(Strng, Strng)>,
	#[serde(default)]
	pub remove: Vec<Strng>,
	#[serde(default)]
	pub body: Option<Strng>,
}

impl TryFrom<LocalTransform> for TransformerConfig {
	type Error = anyhow::Error;

	fn try_from(req: LocalTransform) -> Result<Self, Self::Error> {
		let set = req
			.set
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderName::try_from(k.as_str())?;
				let tv = cel::Expression::new(v.as_str())?;
				Ok::<_, anyhow::Error>((tk, tv))
			})
			.collect::<Result<_, _>>()?;
		let add = req
			.add
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderName::try_from(k.as_str())?;
				let tv = cel::Expression::new(v.as_str())?;
				Ok::<_, anyhow::Error>((tk, tv))
			})
			.collect::<Result<_, _>>()?;
		let remove = req
			.remove
			.into_iter()
			.map(|k| HeaderName::try_from(k.as_str()))
			.collect::<Result<_, _>>()?;
		let body = req
			.body
			.map(|b| cel::Expression::new(b.as_str()))
			.transpose()?;
		Ok(TransformerConfig {
			set,
			add,
			remove,
			body,
		})
	}
}
impl TryFrom<LocalTransformationConfig> for Transformation {
	type Error = anyhow::Error;

	fn try_from(value: LocalTransformationConfig) -> Result<Self, Self::Error> {
		let LocalTransformationConfig { request, response } = value;
		let request = if let Some(req) = request {
			req.try_into()?
		} else {
			Default::default()
		};
		let response = if let Some(resp) = response {
			resp.try_into()?
		} else {
			Default::default()
		};
		Ok(Transformation {
			request: Arc::new(request),
			response: Arc::new(response),
		})
	}
}

#[derive(Clone, Debug, Serialize)]
pub struct Transformation {
	request: Arc<TransformerConfig>,
	response: Arc<TransformerConfig>,
}

impl Transformation {
	pub fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self
			.request
			.add
			.iter()
			.map(|v| &v.1)
			.chain(self.request.set.iter().map(|v| &v.1))
			.chain(self.request.body.as_ref())
			.chain(self.response.add.iter().map(|v| &v.1))
			.chain(self.response.set.iter().map(|v| &v.1))
			.chain(self.response.body.as_ref())
	}
}

#[serde_as]
#[derive(Debug, Default, Serialize)]
pub struct TransformerConfig {
	#[serde_as(serialize_as = "serde_with::Map<SerAsStr, _>")]
	pub add: Vec<(HeaderName, cel::Expression)>,
	#[serde_as(serialize_as = "serde_with::Map<SerAsStr, _>")]
	pub set: Vec<(HeaderName, cel::Expression)>,
	#[serde_as(serialize_as = "Vec<SerAsStr>")]
	pub remove: Vec<HeaderName>,
	pub body: Option<cel::Expression>,
}

pub struct SerAsStr;
impl<T> SerializeAs<T> for SerAsStr
where
	T: AsRef<str>,
{
	fn serialize_as<S>(source: &T, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		source.as_ref().serialize(serializer)
	}
}

fn eval_header_value(exec: &Executor, expr: &Expression) -> anyhow::Result<HeaderValue> {
	Ok(match exec.eval(expr) {
		Ok(cel_interpreter::Value::String(b)) => HeaderValue::from_str(b.as_str())?,
		Ok(cel_interpreter::Value::Bytes(b)) => HeaderValue::from_bytes(b.as_slice())?,
		// Probably we could support this by parsing it
		Ok(v) => anyhow::bail!("invalid response type: {v:?}"),
		Err(e) => anyhow::bail!("invalid response: {}", e),
	})
}

fn eval_body(exec: &Executor, expr: &Expression) -> anyhow::Result<Bytes> {
	let v = exec.eval(expr)?;
	let j = match v.json() {
		Ok(val) => val,
		Err(e) => return Err(anyhow::anyhow!("JSON conversion failed: {}", e)),
	};
	let v = serde_json::to_vec(&j)?;
	Ok(Bytes::copy_from_slice(&v))
}

impl Transformation {
	pub fn apply_request(
		&self,
		req: &mut crate::http::Request,
		ctx: &ContextBuilder,
	) -> anyhow::Result<()> {
		let (mut parts, mut body) = std::mem::take(req).into_parts();
		let res = Self::apply(&mut parts.headers, &mut body, self.request.as_ref(), ctx);
		*req = http::Request::from_parts(parts, body);
		res
	}
	pub fn apply_response(
		&self,
		req: &mut crate::http::Response,
		ctx: &ContextBuilder,
	) -> anyhow::Result<()> {
		let (mut parts, mut body) = std::mem::take(req).into_parts();
		let res = Self::apply(&mut parts.headers, &mut body, self.response.as_ref(), ctx);
		*req = http::Response::from_parts(parts, body);
		res
	}
	fn apply(
		headers: &mut crate::http::HeaderMap,
		body: &mut http::Body,
		cfg: &TransformerConfig,
		ctx: &ContextBuilder,
	) -> anyhow::Result<()> {
		let exec = ctx.build()?;
		for (k, v) in &cfg.add {
			// If it fails, skip the header
			if let Ok(v) = eval_header_value(&exec, v) {
				headers.append(k.clone(), v);
			} else {
				// Need to sanitize it, so a failed execution cannot mean the user can set arbitrary headers.
				headers.remove(k);
			}
		}
		for (k, v) in &cfg.set {
			if let Ok(v) = eval_header_value(&exec, v) {
				headers.insert(k.clone(), v);
			}
		}
		for k in &cfg.remove {
			headers.remove(k);
		}
		if let Some(b) = &cfg.body {
			// If it fails, set an empty body
			let b = eval_body(&exec, b).unwrap_or_default();
			*body = http::Body::from(b);
			headers.remove(&header::CONTENT_LENGTH);
		}
		Ok(())
	}
}

#[cfg(test)]
#[path = "transformation_cel_tests.rs"]
mod tests;
