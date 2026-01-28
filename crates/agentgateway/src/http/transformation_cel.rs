use std::str::FromStr;

use ::http::{HeaderName, header};
use agent_core::prelude::Strng;
use serde_with::{DeserializeAs, SerializeAs, serde_as};

use crate::cel::{Expression, RequestSnapshot};
use crate::http::{HeaderOrPseudo, HeaderOrPseudoValue, RequestOrResponse};
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

fn eval_body(
	r: &RequestOrResponse,
	expr: &Expression,
	request: Option<&cel::RequestSnapshot>,
) -> anyhow::Result<Bytes> {
	match r {
		RequestOrResponse::Request(r) => {
			let exec = cel::Executor::new_request(r);
			let v = exec.eval(expr)?;
			cel::value_as_byte_or_json(v)
		},
		RequestOrResponse::Response(r) => {
			let exec = cel::Executor::new_response(request, r);
			let v = exec.eval(expr)?;
			cel::value_as_byte_or_json(v)
		},
	}
}

impl Transformation {
	pub fn apply_request(&self, req: &mut crate::http::Request) {
		Self::apply(req.into(), self.request.as_ref(), None)
	}

	pub fn apply_response(
		&self,
		resp: &mut crate::http::Response,
		request: Option<&RequestSnapshot>,
	) {
		Self::apply(resp.into(), self.response.as_ref(), request)
	}

	fn exec_header<'a>(
		r: &RequestOrResponse<'a>,
		expr: &'a cel::Expression,
		k: &HeaderOrPseudo,
		request: Option<&'a RequestSnapshot>,
	) -> Option<HeaderOrPseudoValue> {
		match r {
			RequestOrResponse::Request(r) => {
				let exec = cel::Executor::new_request(r);
				let v = exec.eval(expr).ok();
				HeaderOrPseudoValue::from_cel_result(k, v)
			},
			RequestOrResponse::Response(r) => {
				let exec = cel::Executor::new_response(request, r);
				let v = exec.eval(expr).ok();
				HeaderOrPseudoValue::from_cel_result(k, v)
			},
		}
	}

	fn apply<'a>(
		mut r: RequestOrResponse<'a>,
		cfg: &TransformerConfig,
		request: Option<&'a RequestSnapshot>,
	) {
		for (k, v) in &cfg.add {
			let val = Self::exec_header(&r, v, k, request);
			r.apply_header(k, val, true);
		}
		for (k, v) in &cfg.set {
			let val = Self::exec_header(&r, v, k, request);
			r.apply_header(k, val, false);
		}
		for k in &cfg.remove {
			r.headers().remove(k);
		}
		if let Some(b) = &cfg.body {
			// If it fails, set an empty body
			let b = eval_body(&r, b, request).unwrap_or_default();
			*r.body() = http::Body::from(b);
			r.headers().remove(&header::CONTENT_LENGTH);
		}
	}
}

#[cfg(test)]
#[path = "transformation_cel_tests.rs"]
mod tests;
