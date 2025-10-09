use crate::cel::{Executor, Expression};
use crate::{cel, *};
use ::http::StatusCode;
use ::http::header::InvalidHeaderName;
use ::http::uri::{Authority, PathAndQuery, Scheme};
use ::http::{HeaderName, HeaderValue, header};
use agent_core::prelude::Strng;
use cel::Value;
use serde_with::{SerializeAs, serde_as};

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

impl TryFrom<LocalTransform> for TransformerConfig {
	type Error = anyhow::Error;

	fn try_from(req: LocalTransform) -> Result<Self, Self::Error> {
		let set = req
			.set
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderOrPseudo::try_from(k.as_str())?;
				let tv = cel::Expression::new(v.as_str())?;
				Ok::<_, anyhow::Error>((tk, tv))
			})
			.collect::<Result<_, _>>()?;
		let add = req
			.add
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderOrPseudo::try_from(k.as_str())?;
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

#[derive(Debug)]
pub enum HeaderOrPseudo {
	Header(HeaderName),
	Method,
	Scheme,
	Authority,
	Path,
	Status,
}

impl TryFrom<&str> for HeaderOrPseudo {
	type Error = InvalidHeaderName;

	fn try_from(value: &str) -> Result<Self, Self::Error> {
		match value {
			":method" => Ok(HeaderOrPseudo::Method),
			":scheme" => Ok(HeaderOrPseudo::Scheme),
			":authority" => Ok(HeaderOrPseudo::Authority),
			":path" => Ok(HeaderOrPseudo::Path),
			":status" => Ok(HeaderOrPseudo::Status),
			_ => HeaderName::try_from(value).map(HeaderOrPseudo::Header),
		}
	}
}

impl Serialize for HeaderOrPseudo {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self {
			HeaderOrPseudo::Header(h) => h.as_str().serialize(serializer),
			HeaderOrPseudo::Method => ":method".serialize(serializer),
			HeaderOrPseudo::Scheme => ":scheme".serialize(serializer),
			HeaderOrPseudo::Authority => ":authority".serialize(serializer),
			HeaderOrPseudo::Path => ":path".serialize(serializer),
			HeaderOrPseudo::Status => ":status".serialize(serializer),
		}
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
					match k {
						HeaderOrPseudo::Method => {
							if let Ok(m) = http::Method::from_bytes(b) {
								*r.method_mut() = m;
							}
						},
						HeaderOrPseudo::Scheme => {
							if let Ok(s) = Scheme::try_from(b) {
								let _ = http::modify_req_uri(r, |uri| {
									uri.scheme = Some(s);
									Ok(())
								});
							}
						},
						HeaderOrPseudo::Authority => {
							if let Ok(s) = Authority::try_from(b) {
								let _ = http::modify_req_uri(r, |uri| {
									uri.authority = Some(s);
									Ok(())
								});
							}
						},
						HeaderOrPseudo::Path => {
							if let Ok(s) = PathAndQuery::try_from(b) {
								let _ = http::modify_req_uri(r, |uri| {
									uri.path_and_query = Some(s);
									Ok(())
								});
							}
						},
						_ => {},
					}
				}
			},
			_ => {},
		}
	}
}

impl Transformation {
	pub fn apply_request(&self, req: &mut crate::http::Request, exec: &cel::Executor<'_>) {
		Self::apply(req.into(), self.request.as_ref(), exec)
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
