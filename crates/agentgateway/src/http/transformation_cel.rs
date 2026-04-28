use std::str::FromStr;

use ::http::{HeaderName, header};
use agent_core::prelude::Strng;
use serde_with::{DeserializeAs, SerializeAs, serde_as};

use crate::cel::{Expression, RequestSnapshot};
use crate::http::{
	HeaderOrPseudo, HeaderOrPseudoValue, PolicyResponse, Request, RequestOrResponse, Response,
};
use crate::proxy::ProxyResponse;
use crate::telemetry::log::RequestLog;
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
	#[serde(default)]
	#[serde_as(as = "serde_with::Map<_, _>")]
	pub metadata: Vec<(Strng, Strng)>,
}

#[apply(schema!)]
#[derive(Default, ::cel::DynamicType)]
pub struct TransformationMetadata(pub serde_json::Map<String, serde_json::Value>);

impl TransformerConfig {
	fn try_from_local_config<F>(
		req: LocalTransform,
		strict: bool,
		warnings: &mut F,
	) -> anyhow::Result<Self>
	where
		F: FnMut(&str, &cel::Error),
	{
		fn compile<F>(s: &str, strict: bool, warnings: &mut F) -> anyhow::Result<cel::Expression>
		where
			F: FnMut(&str, &cel::Error),
		{
			if strict {
				Ok(cel::Expression::new_strict(s)?)
			} else {
				let (expression, err) = cel::Expression::new_permissive(s);
				if let Some(err) = &err {
					warnings(s, err);
				}
				Ok(expression)
			}
		}

		let set = req
			.set
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderOrPseudo::try_from(k.as_str())?;
				let tv = compile(v.as_str(), strict, warnings)?;
				Ok::<_, anyhow::Error>((tk, tv))
			})
			.collect::<Result<_, _>>()?;
		let add = req
			.add
			.into_iter()
			.map(|(k, v)| {
				let tk = HeaderOrPseudo::try_from(k.as_str())?;
				let tv = compile(v.as_str(), strict, warnings)?;
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
			.map(|b| compile(b.as_str(), strict, warnings))
			.transpose()?;
		let metadata = req
			.metadata
			.into_iter()
			.map(|(k, v)| Ok::<_, anyhow::Error>((k, compile(v.as_str(), strict, warnings)?)))
			.collect::<Result<_, _>>()?;
		Ok(TransformerConfig {
			set,
			add,
			remove,
			body,
			metadata,
		})
	}
}

impl Transformation {
	pub fn try_from_local_config(
		value: LocalTransformationConfig,
		strict: bool,
	) -> anyhow::Result<Self> {
		Self::try_from_local_config_with_warnings(value, strict, |_, _| {})
	}

	pub fn try_from_local_config_with_warnings<F>(
		value: LocalTransformationConfig,
		strict: bool,
		mut warnings: F,
	) -> anyhow::Result<Self>
	where
		F: FnMut(&str, &cel::Error),
	{
		let LocalTransformationConfig { request, response } = value;
		let request = if let Some(req) = request {
			TransformerConfig::try_from_local_config(req, strict, &mut warnings)?
		} else {
			Default::default()
		};
		let response = if let Some(resp) = response {
			TransformerConfig::try_from_local_config(resp, strict, &mut warnings)?
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

#[serde_as]
#[derive(Debug, Default, Serialize)]
pub struct TransformerConfig {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub add: Vec<(HeaderOrPseudo, cel::Expression)>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub set: Vec<(HeaderOrPseudo, cel::Expression)>,
	#[serde_as(serialize_as = "Vec<SerAsStr>")]
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub remove: Vec<HeaderName>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub body: Option<cel::Expression>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub metadata: Vec<(Strng, cel::Expression)>,
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

fn eval_metadata(
	r: &RequestOrResponse,
	expr: &Expression,
	request: Option<&cel::RequestSnapshot>,
) -> anyhow::Result<serde_json::Value> {
	match r {
		RequestOrResponse::Request(r) => {
			let exec = cel::Executor::new_request(r);
			exec
				.eval(expr)
				.and_then(|v| v.json().map_err(|e| cel::Error::Variable(e.to_string())))
				.map_err(anyhow::Error::from)
		},
		RequestOrResponse::Response(r) => {
			let exec = cel::Executor::new_response(request, r);
			exec
				.eval(expr)
				.and_then(|v| v.json().map_err(|e| cel::Error::Variable(e.to_string())))
				.map_err(anyhow::Error::from)
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
		if !cfg.metadata.is_empty() {
			for (name, expr) in &cfg.metadata {
				if let Ok(v) = eval_metadata(&r, expr, request) {
					let metadata = Self::get_meta(&mut r);
					metadata.0.insert(name.to_string(), v);
				}
			}
		}
		for (k, v) in &cfg.add {
			let val = Self::exec_header(&r, v, k, request);
			r.apply_header(k, val, http::HeaderMutationAction::AppendIfExistsOrAdd);
		}
		for (k, v) in &cfg.set {
			let val = Self::exec_header(&r, v, k, request);
			r.apply_header(k, val, http::HeaderMutationAction::OverwriteIfExistsOrAdd);
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

	fn get_meta<'a>(r: &'a mut RequestOrResponse<'_>) -> &'a mut TransformationMetadata {
		let ext = match r {
			RequestOrResponse::Request(req) => req.extensions_mut(),
			RequestOrResponse::Response(resp) => resp.extensions_mut(),
		};

		if ext.get::<TransformationMetadata>().is_none() {
			ext.insert(TransformationMetadata::default());
		}

		ext
			.get_mut::<TransformationMetadata>()
			.expect("we just put this there!")
	}
}

impl crate::store::RequestPolicyTrait for Transformation {
	async fn apply(
		&self,
		_client: &crate::proxy::httpproxy::PolicyClient,
		_log: &mut crate::telemetry::log::RequestLog,
		req: &mut crate::http::Request,
	) -> Result<crate::http::PolicyResponse, crate::proxy::ProxyResponse> {
		self.apply_request(req);
		Ok(crate::http::PolicyResponse::default())
	}

	fn expressions(&self) -> impl Iterator<Item = &Expression> {
		self
			.request
			.add
			.iter()
			.map(|v| &v.1)
			.chain(self.request.set.iter().map(|v| &v.1))
			.chain(self.request.body.as_ref())
			.chain(self.request.metadata.iter().map(|v| &v.1))
			.chain(self.response.add.iter().map(|v| &v.1))
			.chain(self.response.set.iter().map(|v| &v.1))
			.chain(self.response.body.as_ref())
			.chain(self.response.metadata.iter().map(|v| &v.1))
	}
}

impl store::BackendPolicyTrait for Transformation {
	async fn apply(
		&self,
		_log: &mut Option<&mut RequestLog>,
		req: &mut Request,
	) -> Result<PolicyResponse, ProxyResponse> {
		self.apply_request(req);
		Ok(crate::http::PolicyResponse::default())
	}
}

impl store::ResponsePolicyTrait for Transformation {
	async fn apply(
		&self,
		log: &mut RequestLog,
		resp: &mut Response,
	) -> Result<PolicyResponse, ProxyResponse> {
		self.apply_response(resp, log.request_snapshot.as_ref());
		Ok(crate::http::PolicyResponse::default())
	}
}

#[cfg(test)]
#[path = "transformation_cel_tests.rs"]
mod tests;
