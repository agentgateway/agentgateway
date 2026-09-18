use ::http::{HeaderName, HeaderValue};
use agent_core::prelude::Strng;
use serde_with::serde_as;
use tracing::debug;

use crate::cel::{Expression, RequestSnapshot};
use crate::http::{
	HeaderOrPseudo, HeaderOrPseudoValue, PolicyResponse, Request, RequestOrResponse, Response,
};
use crate::proxy::ProxyResponse;
use crate::proxy::httpproxy::PolicyClient;
use crate::telemetry::log::RequestLog;
use crate::{cel, *};

#[apply(schema!)]
#[derive(Default, ::cel::DynamicType)]
pub struct TransformationMetadata(pub serde_json::Map<String, serde_json::Value>);

#[apply(schema_de!)]
#[derive(Default, Serialize)]
#[cfg_attr(feature = "schema", schemars(rename = "LocalTransformationConfig"))]
pub struct Transformation {
	/// Transform the request before it is forwarded.
	#[serde(default)]
	pub request: Option<Arc<TransformerConfig>>,
	/// Transform the response before it is returned.
	#[serde(default)]
	pub response: Option<Arc<TransformerConfig>>,
}

#[serde_as]
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename = "LocalTransform"))]
pub struct TransformerConfig {
	/// Headers to append using CEL expressions for values.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde_as(deserialize_as = "serde_with::Map<_, _>")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::BTreeMap<String, String>")
	)]
	pub add: Vec<(HeaderOrPseudo, cel::Expression)>,
	/// Headers to set using CEL expressions for values.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde_as(deserialize_as = "serde_with::Map<_, _>")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::BTreeMap<String, String>")
	)]
	pub set: Vec<(HeaderOrPseudo, cel::Expression)>,
	/// Header names to remove.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde_as(as = "Vec<crate::serdes::SerAsStr>")]
	#[cfg_attr(feature = "schema", schemars(with = "Vec<String>"))]
	pub remove: Vec<HeaderName>,
	/// CEL expression that computes the full set of headers, replacing all existing headers.
	/// The expression must evaluate to a map of header name to value (a string, or a list of
	/// strings for a repeated header). Pseudo-headers (`:method`, `:path`, etc.) are ignored;
	/// set those explicitly with `set`/`add`. `replace` is applied before `add`/`set`/`remove`,
	/// so those still operate on top of the replaced headers.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub replace: Option<cel::Expression>,
	/// CEL expression that computes a replacement body.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub body: Option<cel::Expression>,
	/// Metadata values to add using CEL expressions.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	#[serde_as(deserialize_as = "serde_with::Map<_, _>")]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::BTreeMap<String, String>")
	)]
	pub metadata: Vec<(Strng, cel::Expression)>,
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

fn eval_headers(
	r: &RequestOrResponse,
	expr: &Expression,
	request: Option<&cel::RequestSnapshot>,
) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
	match eval_metadata(r, expr, request)? {
		serde_json::Value::Object(map) => Ok(map),
		other => anyhow::bail!("replace expression must evaluate to a map, got {other}"),
	}
}

fn json_to_header_value(v: &serde_json::Value) -> Option<HeaderValue> {
	match v {
		serde_json::Value::String(s) => HeaderValue::from_str(s).ok(),
		serde_json::Value::Number(n) => HeaderValue::from_str(&n.to_string()).ok(),
		serde_json::Value::Bool(b) => HeaderValue::from_str(&b.to_string()).ok(),
		serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => None,
	}
}

impl Transformation {
	pub fn apply_request(&self, req: &mut crate::http::Request) {
		if let Some(config) = &self.request {
			Self::apply(req.into(), config, None);
		}
	}

	pub fn apply_response(
		&self,
		resp: &mut crate::http::Response,
		request: Option<&RequestSnapshot>,
	) {
		if let Some(request_metadata) = request.and_then(|req| req.metadata.as_ref()) {
			// Transformation metadata is currently stored in request/response extensions.
			// Seed request metadata into the response extension so response-phase CEL,
			// response snapshots, and log CEL all see one accumulated metadata map.
			// Keep existing response keys so response metadata wins on conflicts.
			let ext = resp.extensions_mut();
			if let Some(response_metadata) = ext.get_mut::<TransformationMetadata>() {
				for (key, value) in &request_metadata.0 {
					response_metadata
						.0
						.entry(key.clone())
						.or_insert_with(|| value.clone());
				}
			} else {
				ext.insert(request_metadata.clone());
			}
		}
		if let Some(config) = &self.response {
			Self::apply(resp.into(), config, request);
		}
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
		if let Some(expr) = &cfg.replace {
			match eval_headers(&r, expr, request) {
				Ok(headers) => {
					// Replace the full header set. Do this before add/set/remove so those still
					// operate on top of the replaced headers.
					r.headers().clear();
					for (name, value) in headers {
						// Only real headers are replaced; pseudo-headers must be set via set/add.
						let Ok(HeaderOrPseudo::Header(name)) = HeaderOrPseudo::try_from(name.as_str()) else {
							continue;
						};
						match value {
							serde_json::Value::Array(items) => {
								for item in &items {
									if let Some(hv) = json_to_header_value(item) {
										r.headers().append(name.clone(), hv);
									}
								}
							},
							other => {
								if let Some(hv) = json_to_header_value(&other) {
									r.headers().insert(name.clone(), hv);
								}
							},
						}
					}
				},
				// On evaluation error (or a non-map result), leave headers untouched rather than
				// dropping every header because of a transient failure.
				Err(e) => {
					debug!("transformation replace expression did not produce a header map: {e}");
				},
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
			r.replace_body_bytes(b);
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
			.iter()
			.chain(self.response.iter())
			.flat_map(|config| {
				config
					.add
					.iter()
					.map(|v| &v.1)
					.chain(config.set.iter().map(|v| &v.1))
					.chain(config.replace.as_ref())
					.chain(config.body.as_ref())
					.chain(config.metadata.iter().map(|v| &v.1))
			})
	}
}

impl store::BackendPolicyTrait for Transformation {
	async fn apply(
		&self,
		_client: &PolicyClient,
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
		self.apply_response(resp, log.request_snapshot.as_deref());
		Ok(crate::http::PolicyResponse::default())
	}
}

#[cfg(test)]
#[path = "transformation_cel_tests.rs"]
mod tests;
