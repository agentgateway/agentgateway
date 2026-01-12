use std::str::FromStr;

use ::http::{HeaderName, HeaderValue, StatusCode, header};
use agent_core::prelude::Strng;
use cel::Value;
use serde_with::{DeserializeAs, SerializeAs, serde_as};

use crate::cel::{Executor, Expression};
use crate::http::HeaderOrPseudo;
use crate::{cel, *};

#[derive(Default)]
#[apply(schema_de!)]
pub struct LocalTransformationConfig {
	#[serde(default)]
	pub request: Option<LocalTransform>,
	#[serde(default)]
	pub response: Option<LocalTransform>,
}

#[derive(Default)]
#[apply(schema_de!)]
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

impl TransformerConfig {
	fn try_from_local_config(req: LocalTransform, strict: bool) -> anyhow::Result<Self> {
		let compile = |s: &str| {
			if strict {
				cel::Expression::new_strict(s)
			} else {
				Ok(cel::Expression::new_permissive(s))
			}
		};
		let set = req
			.set
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderOrPseudo::try_from(k.as_str())?;
				let tv = compile(v.as_str())?;
				Ok::<_, anyhow::Error>((tk, tv))
			})
			.collect::<Result<_, _>>()?;
		let add = req
			.add
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderOrPseudo::try_from(k.as_str())?;
				let tv = compile(v.as_str())?;
				Ok::<_, anyhow::Error>((tk, tv))
			})
			.collect::<Result<_, _>>()?;
		let remove = req
			.remove
			.into_iter()
			.map(|k| HeaderName::try_from(k.as_str()))
			.collect::<Result<_, _>>()?;
		let body = req.body.map(|b| compile(b.as_str())).transpose()?;
		Ok(TransformerConfig {
			set,
			add,
			remove,
			body,
		})
	}
}

impl Transformation {
	pub fn try_from_local_config(
		value: LocalTransformationConfig,
		strict: bool,
	) -> anyhow::Result<Self> {
		let LocalTransformationConfig { request, response } = value;
		let request = if let Some(req) = request {
			TransformerConfig::try_from_local_config(req, strict)?
		} else {
			Default::default()
		};
		let response = if let Some(resp) = response {
			TransformerConfig::try_from_local_config(resp, strict)?
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
	pub add: Vec<(HeaderOrPseudo, cel::Expression)>,
	pub set: Vec<(HeaderOrPseudo, cel::Expression)>,
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
impl<'de, T> DeserializeAs<'de, T> for SerAsStr
where
	T: FromStr,
	<T as FromStr>::Err: std::fmt::Display,
{
	fn deserialize_as<D>(deserializer: D) -> Result<T, D::Error>
	where
		D: Deserializer<'de>,
	{
		let s = <&str>::deserialize(deserializer)?;
		s.parse().map_err(serde::de::Error::custom)
	}
}

fn eval_body(exec: &Executor, expr: &Expression) -> anyhow::Result<Bytes> {
	let v = exec.eval(expr)?;
	match &v {
		Value::String(s) => return Ok(Bytes::copy_from_slice(s.as_bytes())),
		Value::Bytes(b) => return Ok(Bytes::copy_from_slice(b)),
		_ => {},
	}
	let j = match v.json() {
		Ok(val) => val,
		Err(e) => return Err(anyhow::anyhow!("JSON conversion failed: {}", e)),
	};
	let v = serde_json::to_vec(&j)?;
	Ok(Bytes::copy_from_slice(&v))
}

#[derive(Debug)]
enum RequestOrResponse<'a> {
	Request(&'a mut http::Request),
	Response(&'a mut http::Response),
}

impl<'a> From<&'a mut http::Request> for RequestOrResponse<'a> {
	fn from(req: &'a mut http::Request) -> Self {
		RequestOrResponse::Request(req)
	}
}

impl<'a> From<&'a mut http::Response> for RequestOrResponse<'a> {
	fn from(req: &'a mut http::Response) -> RequestOrResponse<'a> {
		RequestOrResponse::Response(req)
	}
}

impl<'a> RequestOrResponse<'a> {
	pub fn headers(&mut self) -> &mut http::HeaderMap {
		match self {
			RequestOrResponse::Request(r) => r.headers_mut(),
			RequestOrResponse::Response(r) => r.headers_mut(),
		}
	}
	fn body(&mut self) -> &mut http::Body {
		match self {
			RequestOrResponse::Request(r) => r.body_mut(),
			RequestOrResponse::Response(r) => r.body_mut(),
		}
	}
	fn add_header(&mut self, k: &HeaderOrPseudo, res: Option<Value>, append: bool) {
		match (res, k) {
			(res, HeaderOrPseudo::Header(h)) => {
				if let Some(v) = res
					.as_ref()
					.and_then(cel::value_as_bytes)
					.and_then(|b| HeaderValue::from_bytes(b).ok())
				{
					if append {
						self.headers().append(h.clone(), v);
					} else {
						self.headers().insert(h.clone(), v);
					}
				} else {
					// Need to sanitize it, so a failed execution cannot mean the user can set arbitrary headers.
					self.headers().remove(h);
				}
			},
			(Some(v), HeaderOrPseudo::Status) => {
				if let RequestOrResponse::Response(r) = self
					&& let Some(b) = cel::value_as_int(&v)
					&& let Ok(b) = u16::try_from(b)
					&& let Ok(s) = StatusCode::from_u16(b)
				{
					*r.status_mut() = s
				}
			},
			(Some(v), _) => {
				if let RequestOrResponse::Request(r) = self
					&& let Some(b) = cel::value_as_bytes(&v)
				{
					let mut rr = crate::http::RequestOrResponse::Request(r);
					let _ = crate::http::apply_header_or_pseudo(&mut rr, k, b);
				}
			},
			_ => {},
		}
	}
}

impl Transformation {
	pub fn has_request(&self) -> bool {
		self.request.body.is_some()
			|| !self.request.add.is_empty()
			|| !self.request.set.is_empty()
			|| !self.request.remove.is_empty()
	}
	pub fn apply_request(&self, req: &mut crate::http::Request, exec: &cel::Executor<'_>) {
		Self::apply(req.into(), self.request.as_ref(), exec)
	}

	pub fn has_response(&self) -> bool {
		self.response.body.is_some()
			|| !self.response.add.is_empty()
			|| !self.response.set.is_empty()
			|| !self.response.remove.is_empty()
	}

	pub fn apply_response(&self, resp: &mut crate::http::Response, exec: &cel::Executor<'_>) {
		Self::apply(resp.into(), self.response.as_ref(), exec)
	}

	fn apply<'a>(mut r: RequestOrResponse<'a>, cfg: &TransformerConfig, exec: &cel::Executor<'_>) {
		for (k, v) in &cfg.add {
			r.add_header(k, exec.eval(v).ok(), true);
		}
		for (k, v) in &cfg.set {
			r.add_header(k, exec.eval(v).ok(), false);
		}
		for k in &cfg.remove {
			r.headers().remove(k);
		}
		if let Some(b) = &cfg.body {
			// If it fails, set an empty body
			let b = eval_body(exec, b).unwrap_or_default();
			*r.body() = http::Body::from(b);
			r.headers().remove(&header::CONTENT_LENGTH);
		}
	}
}

#[cfg(test)]
#[path = "transformation_cel_tests.rs"]
mod tests;
