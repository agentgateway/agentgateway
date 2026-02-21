use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ::http::header::{HeaderName, HeaderValue};
use headers::HeaderMapExt;
use http::Method;
use http::header::{ACCEPT, CONTENT_LENGTH, CONTENT_TYPE, HOST, TRANSFER_ENCODING};
use once_cell::sync::Lazy;
use openapiv3::{OpenAPI, Parameter, ReferenceOr, RequestBody, Schema, SchemaKind, Type};
use percent_encoding::{AsciiSet, utf8_percent_encode};
use regex::{Captures, Regex, Replacer};
use rmcp::model::{ClientRequest, JsonObject, JsonRpcRequest, Tool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::mcp::mergestream;
use crate::mcp::mergestream::Messages;
use crate::mcp::upstream::{IncomingRequestContext, UpstreamError};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct UpstreamOpenAPICall {
	pub method: String, /* TODO: Switch to Method, but will require getting rid of Serialize/Deserialize */
	pub path: String,
	pub allowed_headers: HashSet<String>,
	// todo: params
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
	#[error("missing components")]
	MissingComponents,
	#[error("invalid reference: {0}")]
	InvalidReference(String),
	#[error("missing reference")]
	MissingReference(String),
	#[error("unsupported reference")]
	UnsupportedReference(String),
	#[error("information required: {0}")]
	InformationRequired(String),
	#[error("serde error: {0}")]
	SerdeError(#[from] serde_json::Error),
	#[error("io error: {0}")]
	IoError(#[from] std::io::Error),
	#[error("Invalid URL: {0}")]
	InvalidUrl(#[from] url::ParseError),
}

pub(crate) fn get_server_prefix(server: &OpenAPI) -> Result<String, ParseError> {
	match server.servers.len() {
		0 => Ok("".to_string()), // Return empty string instead of "/" to avoid double slash
		1 => Ok(server.servers[0].url.clone()),
		_ => Err(ParseError::UnsupportedReference(format!(
			"multiple servers are not supported: {:?}",
			server.servers
		))),
	}
}

fn resolve_schema<'a>(
	reference: &'a ReferenceOr<Schema>,
	doc: &'a OpenAPI,
) -> Result<&'a Schema, ParseError> {
	match reference {
		ReferenceOr::Reference { reference } => {
			let reference = reference
				.strip_prefix("#/components/schemas/")
				.ok_or(ParseError::InvalidReference(reference.to_string()))?;
			let components: &openapiv3::Components = doc
				.components
				.as_ref()
				.ok_or(ParseError::MissingComponents)?;
			let schema = components
				.schemas
				.get(reference)
				.ok_or(ParseError::MissingReference(reference.to_string()))?;
			resolve_schema(schema, doc)
		},
		ReferenceOr::Item(schema) => Ok(schema),
	}
}

/// Recursively resolves all nested schema references (`$ref`) within a given schema,
/// returning a new `Schema` object with all references replaced by their corresponding items.
fn resolve_nested_schema<'a>(
	reference: &'a ReferenceOr<Schema>,
	doc: &'a OpenAPI,
) -> Result<Schema, ParseError> {
	// 1. Resolve the initial reference to get the base Schema object (immutable borrow)
	let base_schema = resolve_schema(reference, doc)?;

	// 2. Clone the base schema to create a mutable owned version we can modify
	let mut resolved_schema = base_schema.clone();

	// 3. Match on the kind and recursively resolve + update the mutable clone
	match &mut resolved_schema.schema_kind {
		SchemaKind::Type(Type::Object(obj)) => {
			for prop_ref_box in obj.properties.values_mut() {
				let owned_prop_ref_or_box = prop_ref_box.clone();
				let temp_prop_ref = match owned_prop_ref_or_box {
					ReferenceOr::Reference { reference } => ReferenceOr::Reference { reference },
					ReferenceOr::Item(boxed_item) => ReferenceOr::Item((*boxed_item).clone()),
				};
				let resolved_prop = resolve_nested_schema(&temp_prop_ref, doc)?;
				*prop_ref_box = ReferenceOr::Item(Box::new(resolved_prop));
			}
		},
		SchemaKind::Type(Type::Array(arr)) => {
			if let Some(items_ref_box) = arr.items.as_mut() {
				let owned_items_ref_or_box = items_ref_box.clone();
				let temp_items_ref = match owned_items_ref_or_box {
					ReferenceOr::Reference { reference } => ReferenceOr::Reference { reference },
					ReferenceOr::Item(boxed_item) => ReferenceOr::Item((*boxed_item).clone()),
				};
				let resolved_items = resolve_nested_schema(&temp_items_ref, doc)?;
				*items_ref_box = ReferenceOr::Item(Box::new(resolved_items));
			}
		},
		// Handle combiners (OneOf, AllOf, AnyOf) with separate arms
		SchemaKind::OneOf { one_of } => {
			for ref_or_schema in one_of.iter_mut() {
				let temp_ref = ref_or_schema.clone();
				let resolved = resolve_nested_schema(&temp_ref, doc)?;
				*ref_or_schema = ReferenceOr::Item(resolved);
			}
		},
		SchemaKind::AllOf { all_of } => {
			for ref_or_schema in all_of.iter_mut() {
				let temp_ref = ref_or_schema.clone();
				let resolved = resolve_nested_schema(&temp_ref, doc)?;
				*ref_or_schema = ReferenceOr::Item(resolved);
			}
		},
		SchemaKind::AnyOf { any_of } => {
			for ref_or_schema in any_of.iter_mut() {
				let temp_ref = ref_or_schema.clone();
				let resolved = resolve_nested_schema(&temp_ref, doc)?;
				*ref_or_schema = ReferenceOr::Item(resolved);
			}
		},
		SchemaKind::Not { not } => {
			let temp_ref = (**not).clone();
			let resolved = resolve_nested_schema(&temp_ref, doc)?;
			**not = ReferenceOr::Item(resolved);
		},
		SchemaKind::Any(any_schema) => {
			// Properties
			for prop_ref_box in any_schema.properties.values_mut() {
				let owned_prop_ref_or_box = prop_ref_box.clone();
				let temp_prop_ref = match owned_prop_ref_or_box {
					ReferenceOr::Reference { reference } => ReferenceOr::Reference { reference },
					ReferenceOr::Item(boxed_item) => ReferenceOr::Item((*boxed_item).clone()),
				};
				let resolved_prop = resolve_nested_schema(&temp_prop_ref, doc)?;
				*prop_ref_box = ReferenceOr::Item(Box::new(resolved_prop));
			}
			// Items
			if let Some(items_ref_box) = any_schema.items.as_mut() {
				let owned_items_ref_or_box = items_ref_box.clone();
				let temp_items_ref = match owned_items_ref_or_box {
					ReferenceOr::Reference { reference } => ReferenceOr::Reference { reference },
					ReferenceOr::Item(boxed_item) => ReferenceOr::Item((*boxed_item).clone()),
				};
				let resolved_items = resolve_nested_schema(&temp_items_ref, doc)?;
				*items_ref_box = ReferenceOr::Item(Box::new(resolved_items));
			}
			// oneOf, allOf, anyOf
			for vec_ref in [
				&mut any_schema.one_of,
				&mut any_schema.all_of,
				&mut any_schema.any_of,
			] {
				for ref_or_schema in vec_ref.iter_mut() {
					let temp_ref = ref_or_schema.clone();
					let resolved = resolve_nested_schema(&temp_ref, doc)?;
					*ref_or_schema = ReferenceOr::Item(resolved);
				}
			}
			// not
			if let Some(not_box) = any_schema.not.as_mut() {
				let temp_ref = (**not_box).clone();
				let resolved = resolve_nested_schema(&temp_ref, doc)?;
				**not_box = ReferenceOr::Item(resolved);
			}
		},
		// Base types (String, Number, Integer, Boolean) - no nested schemas to resolve further
		SchemaKind::Type(_) => {}, // Do nothing, already resolved.
	}

	// 4. Return the modified owned schema
	Ok(resolved_schema)
}

fn resolve_parameter<'a>(
	reference: &'a ReferenceOr<Parameter>,
	doc: &'a OpenAPI,
) -> Result<&'a Parameter, ParseError> {
	match reference {
		ReferenceOr::Reference { reference } => {
			let reference = reference
				.strip_prefix("#/components/parameters/")
				.ok_or(ParseError::MissingReference(reference.to_string()))?;
			let components: &openapiv3::Components = doc
				.components
				.as_ref()
				.ok_or(ParseError::MissingComponents)?;
			let parameter = components
				.parameters
				.get(reference)
				.ok_or(ParseError::MissingReference(reference.to_string()))?;
			resolve_parameter(parameter, doc)
		},
		ReferenceOr::Item(parameter) => Ok(parameter),
	}
}

fn resolve_request_body<'a>(
	reference: &'a ReferenceOr<RequestBody>,
	doc: &'a OpenAPI,
) -> Result<&'a RequestBody, ParseError> {
	match reference {
		ReferenceOr::Reference { reference } => {
			let reference = reference
				.strip_prefix("#/components/requestBodies/")
				.ok_or(ParseError::MissingReference(reference.to_string()))?;
			let components: &openapiv3::Components = doc
				.components
				.as_ref()
				.ok_or(ParseError::MissingComponents)?;
			let request_body = components
				.request_bodies
				.get(reference)
				.ok_or(ParseError::MissingReference(reference.to_string()))?;
			resolve_request_body(request_body, doc)
		},
		ReferenceOr::Item(request_body) => Ok(request_body),
	}
}

/// We need to rework this and I don't want to forget.
///
/// We need to be able to handle data which can end up in multiple destinations:
/// 1. Headers
/// 2. Body
/// 3. Query Params
/// 4. Templated Path Params
///
/// To support this we should create a nested JSON schema which has each of them.
/// That way the client code can properly separate the objects passed by the client.
pub(crate) fn parse_openapi_schema(
	open_api: &OpenAPI,
) -> Result<Vec<(Tool, UpstreamOpenAPICall)>, ParseError> {
	let tool_defs: Result<Vec<_>, _> = open_api
		.paths
		.iter()
		.map(
			|(path, path_info)| -> Result<Vec<(Tool, UpstreamOpenAPICall)>, ParseError> {
				let item = path_info
					.as_item()
					.ok_or(ParseError::UnsupportedReference(path.to_string()))?;
				let items: Result<Vec<_>, _> = item
					.iter()
					.map(
						|(method, op)| -> Result<(Tool, UpstreamOpenAPICall), ParseError> {
							let name = op
								.operation_id
								.clone()
								.ok_or(ParseError::InformationRequired(format!(
									"operation_id is required for {path}"
								)))?;

							// Build the schema
							let mut final_schema = JsonSchema::default();

							let body: Option<(String, serde_json::Value, bool)> = match op.request_body.as_ref() {
								Some(body) => {
									let body = resolve_request_body(body, open_api)?;
									match body.content.get("application/json") {
										Some(media_type) => {
											let schema_ref = media_type
												.schema
												.as_ref()
												.ok_or(ParseError::MissingReference("application/json".to_string()))?;
											let schema = resolve_nested_schema(schema_ref, open_api)?;
											let body_schema =
												serde_json::to_value(schema).map_err(ParseError::SerdeError)?;
											final_schema
												.properties
												.insert(BODY_NAME.clone(), body_schema.clone());
											Some((BODY_NAME.clone(), body_schema, body.required))
										},
										None => None,
									}
								},
								None => None,
							};

							if let Some((name, schema, required)) = body {
								if required {
									final_schema.required.push(name.clone());
								}
								final_schema.properties.insert(name.clone(), schema.clone());
							}

							let mut param_schemas: HashMap<ParameterType, Vec<(String, JsonObject, bool)>> =
								HashMap::new();
							op.parameters
								.iter()
								.try_for_each(|p| -> Result<(), ParseError> {
									let item = resolve_parameter(p, open_api)?;
									let (name, schema, required) = build_schema_property(open_api, item)?;
									match item {
										Parameter::Header { .. } => {
											param_schemas
												.entry(ParameterType::Header)
												.or_insert_with(Vec::new)
												.push((name, schema, required));
											Ok(())
										},
										Parameter::Query { .. } => {
											param_schemas
												.entry(ParameterType::Query)
												.or_insert_with(Vec::new)
												.push((name, schema, required));
											Ok(())
										},
										Parameter::Path { .. } => {
											param_schemas
												.entry(ParameterType::Path)
												.or_insert_with(Vec::new)
												.push((name, schema, required));
											Ok(())
										},
										_ => Err(ParseError::UnsupportedReference(
											"parameter type COOKIE is not supported".to_string(),
										)),
									}
								})?;

							// Extract allowed header names before consuming param_schemas
							let allowed_headers: HashSet<String> = param_schemas
								.get(&ParameterType::Header)
								.map(|headers| headers.iter().map(|(name, _, _)| name.clone()).collect())
								.unwrap_or_default();

							for (param_type, props) in param_schemas {
								let sub_schema = JsonSchema {
									required: props
										.iter()
										.flat_map(|(name, _, req)| if *req { Some(name.clone()) } else { None })
										.collect(),
									properties: props
										.iter()
										.map(|(name, s, _)| (name.clone(), json!(s)))
										.collect(),
									..Default::default()
								};

								if !sub_schema.required.is_empty() {
									final_schema.required.push(param_type.to_string());
								}
								final_schema
									.properties
									.insert(param_type.to_string(), json!(sub_schema));
							}

							let final_json =
								serde_json::to_value(final_schema).map_err(ParseError::SerdeError)?;
							let final_json = final_json
								.as_object()
								.ok_or(ParseError::UnsupportedReference(
									"final schema is not an object".to_string(),
								))?
								.clone();
							let tool = Tool {
								execution: None,
								meta: None,
								annotations: None,
								name: Cow::Owned(name.clone()),
								description: Some(Cow::Owned(
									op.description
										.as_ref()
										.unwrap_or_else(|| op.summary.as_ref().unwrap_or(&name))
										.to_string(),
								)),
								input_schema: Arc::new(final_json),
								// TODO: support output_schema
								output_schema: None,
								icons: None,
								title: None,
							};
							let upstream = UpstreamOpenAPICall {
								// method: Method::from_bytes(method.as_ref()).expect("todo"),
								method: method.to_string(),
								path: path.clone(),
								allowed_headers,
							};
							Ok((tool, upstream))
						},
					)
					.collect();
				// Rust has a hard time with this...
				let items = items?;
				Ok(items)
			},
		)
		.collect();

	match tool_defs {
		Ok(tool_defs) => Ok(tool_defs.into_iter().flatten().collect()),
		Err(e) => Err(e),
	}
}

// Used to index the parameter types for the schema
lazy_static::lazy_static! {
	pub static ref BODY_NAME: String = "body".to_string();
	pub static ref HEADER_NAME: String = "header".to_string();
	pub static ref QUERY_NAME: String = "query".to_string();
	pub static ref PATH_NAME: String = "path".to_string();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ParameterType {
	Header,
	Query,
	Path,
}

impl std::fmt::Display for ParameterType {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{}",
			match self {
				ParameterType::Header => "header",
				ParameterType::Query => "query",
				ParameterType::Path => "path",
			}
		)
	}
}

fn build_schema_property(
	open_api: &OpenAPI,
	item: &Parameter,
) -> Result<(String, JsonObject, bool), ParseError> {
	let p = item.parameter_data_ref();
	let mut schema = match &p.format {
		openapiv3::ParameterSchemaOrContent::Schema(reference) => {
			let resolved_schema = resolve_schema(reference, open_api)?;
			serde_json::to_value(resolved_schema)
				.map_err(ParseError::SerdeError)?
				.as_object()
				.ok_or(ParseError::UnsupportedReference(format!(
					"parameter {} is not an object",
					p.name
				)))?
				.clone()
		},
		openapiv3::ParameterSchemaOrContent::Content(content) => {
			return Err(ParseError::UnsupportedReference(format!(
				"content is not supported for parameters: {content:?}"
			)));
		},
	};

	if let Some(desc) = &p.description {
		schema.insert("description".to_string(), json!(desc));
	}

	Ok((p.name.clone(), schema, p.required))
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonSchema {
	required: Vec<String>,
	properties: JsonObject,
	r#type: String,
}

impl Default for JsonSchema {
	fn default() -> Self {
		Self {
			required: vec![],
			properties: JsonObject::new(),
			r#type: "object".to_string(),
		}
	}
}

/// Regex to match path template parameters like `{param_name}`.
static PATH_PARAM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{([^}]+)\}").unwrap());

/// Characters that are safe in path segments (RFC 3986 unreserved characters).
/// All other characters will be percent-encoded to prevent path traversal/injection.
const PATH_SEGMENT_SAFE: &AsciiSet = &percent_encoding::NON_ALPHANUMERIC
	.remove(b'-')
	.remove(b'.')
	.remove(b'_')
	.remove(b'~');

/// Replaces path template parameters.
struct PathParamReplacer(serde_json::Map<String, Value>);

impl Replacer for PathParamReplacer {
	fn replace_append(&mut self, caps: &Captures<'_>, dst: &mut String) {
		let param = &caps[1];
		match self.0.get(param) {
			Some(Value::Number(n_val)) => return dst.push_str(&n_val.to_string()),
			Some(Value::String(s_val)) => {
				return dst.extend(utf8_percent_encode(s_val, PATH_SEGMENT_SAFE));
			},
			Some(unexpected) => warn!(
				"Unexpected parameter '{param}' (value: {:?}), leaving path param",
				unexpected
			),
			_ => {},
		};
		// fallback to use path parm
		dst.push_str(&caps[0]);
	}
}

/// Normalizes URL path construction to avoid double slashes
/// Ensures exactly one slash between prefix and path components
fn normalize_url_path(prefix: &str, path: &str) -> String {
	let prefix = prefix.trim_end_matches('/');
	let path = if path.starts_with('/') {
		path
	} else {
		// If path doesn't start with '/', add one
		&format!("/{path}")
	};

	if prefix.is_empty() {
		path.to_string()
	} else {
		format!("{prefix}{path}")
	}
}

fn encode_query_value(value: &str) -> Cow<'_, str> {
	Cow::from(utf8_percent_encode(
		value,
		percent_encoding::NON_ALPHANUMERIC,
	))
}

#[derive(Debug)]
pub struct Handler {
	pub prefix: String,
	pub http_client: super::McpHttpClient,
	pub tools: Vec<(Tool, UpstreamOpenAPICall)>,
}

impl Handler {
	pub fn new(
		http_client: super::McpHttpClient,
		tools: Vec<(Tool, UpstreamOpenAPICall)>,
		prefix: String,
	) -> Self {
		Self {
			prefix,
			http_client,
			tools,
		}
	}

	pub async fn send_message(
		&self,
		request: JsonRpcRequest<ClientRequest>,
		ctx: &IncomingRequestContext,
	) -> Result<mergestream::Messages, UpstreamError> {
		use rmcp::model::*;
		let method = request.request.method();
		let id = request.id;
		let res = match request.request {
			ClientRequest::InitializeRequest(_) => Messages::from_result(
				id,
				ServerInfo {
					capabilities: ServerCapabilities::builder().enable_tools().build(),
					..Default::default()
				},
			),
			ClientRequest::GetPromptRequest(_) => Messages::from_result(
				id,
				GetPromptResult {
					description: None,
					messages: vec![],
				},
			),
			ClientRequest::ListPromptsRequest(_) => Messages::from_result(
				id,
				ListPromptsResult {
					meta: None,
					next_cursor: None,
					prompts: vec![],
				},
			),
			ClientRequest::ListResourcesRequest(_) => Messages::from_result(
				id,
				ListResourcesResult {
					meta: None,
					next_cursor: None,
					resources: vec![],
				},
			),
			ClientRequest::ListResourceTemplatesRequest(_) => Messages::from_result(
				id,
				ListResourceTemplatesResult {
					meta: None,
					next_cursor: None,
					resource_templates: vec![],
				},
			),
			ClientRequest::ListTasksRequest(_) => Messages::from_result(
				id,
				ListTasksResult {
					next_cursor: None,
					tasks: vec![],
					total: None,
				},
			),
			ClientRequest::GetTaskInfoRequest(_) => {
				return Err(UpstreamError::InvalidMethod(method.to_string()));
			},
			ClientRequest::GetTaskResultRequest(_) => {
				return Err(UpstreamError::InvalidMethod(method.to_string()));
			},
			ClientRequest::CancelTaskRequest(_) => {
				return Err(UpstreamError::InvalidMethod(method.to_string()));
			},
			ClientRequest::ReadResourceRequest(_) => {
				Messages::from_result(id, ReadResourceResult { contents: vec![] })
			},
			ClientRequest::PingRequest(_)
			| ClientRequest::CustomRequest(_)
			| ClientRequest::SetLevelRequest(_)
			| ClientRequest::SubscribeRequest(_)
			| ClientRequest::UnsubscribeRequest(_) => Messages::empty(),
			ClientRequest::CompleteRequest(_) => {
				return Err(UpstreamError::InvalidMethod(method.to_string()));
			},
			ClientRequest::CallToolRequest(ctr) => {
				let res = self
					.call_tool(ctr.params.name.as_ref(), ctr.params.arguments, ctx)
					.await?;

				// Serialize structured content to JSON string for backwards compatibility
				// Per MCP spec https://modelcontextprotocol.io/specification/2025-06-18/server/tools#structured-content:
				//   "a tool that returns structured content SHOULD also return the serialized JSON in a TextContent block"
				// Note: This part of the spec is in flux, see https://github.com/modelcontextprotocol/modelcontextprotocol/issues/1624
				let serialized_content = serde_json::to_string(&res)
					.map_err(|e| anyhow::anyhow!("Failed to serialize tool response: {}", e))?;

				Messages::from_result(
					id,
					CallToolResult {
						content: vec![Content::text(serialized_content)],
						structured_content: Some(res),
						is_error: None,
						meta: None,
					},
				)
			},
			ClientRequest::ListToolsRequest(_) => Messages::from_result(
				id,
				ListToolsResult {
					meta: None,
					next_cursor: None,
					tools: self.tools(),
				},
			),
		};
		Ok(res)
	}

	/// We need to use the parse the schema to get the correct args.
	/// They are in the json schema under the "properties" key.
	/// Body is under the "body" key.
	/// Headers are under the "header" key.
	/// Query params are under the "query" key.
	/// Path params are under the "path" key.
	///
	/// Query params need to be added to the url as query params.
	/// Headers need to be added to the request headers.
	/// Body needs to be added to the request body.
	/// Path params need to be added to the template params in the path.
	pub async fn call_tool(
		&self,
		name: &str,
		args: Option<JsonObject>,
		ctx: &IncomingRequestContext,
	) -> Result<serde_json::Value, UpstreamError> {
		let (_tool, info) = self
			.tools
			.iter()
			.find(|(t, _info)| t.name == name)
			.ok_or_else(|| anyhow::anyhow!("tool {} not found", name))?;

		let args = args.unwrap_or_default();

		// --- Parameter Extraction ---
		let path_params = args
			.get(&*PATH_NAME)
			.and_then(Value::as_object)
			.cloned()
			.unwrap_or_default();
		let query_params = args
			.get(&*QUERY_NAME)
			.and_then(Value::as_object)
			.cloned()
			.unwrap_or_default();
		let header_params = args
			.get(&*HEADER_NAME)
			.and_then(Value::as_object)
			.cloned()
			.unwrap_or_default();
		let body_value = args.get(&*BODY_NAME).cloned();

		// --- URL Construction ---
		// Substitute path parameters into the path template in a single pass
		let path = PATH_PARAM_RE.replace_all(&info.path, PathParamReplacer(path_params));

		// Use normalize_url_path to avoid double slashes
		let normalized_path = normalize_url_path(&self.prefix, &path);
		let base_url = format!(
			"{}://{}{}",
			"http",
			self.http_client.backend().hostport(),
			normalized_path
		);

		// --- Request Building ---
		let method = Method::from_bytes(info.method.to_uppercase().as_bytes()).map_err(|e| {
			anyhow::anyhow!(
				"Invalid HTTP method '{}' for tool '{}': {}",
				info.method,
				name,
				e
			)
		})?;

		// Build query string
		let query_string = if query_params.is_empty() {
			String::new()
		} else {
			let format_pair = |param_name: &str, key: &str, v: &Value| -> Option<String> {
				match v {
					Value::Null => Some(key.to_string()),
					Value::Bool(b) => Some(format!("{key}={b}")),
					Value::Number(n) => Some(format!("{key}={n}")),
					Value::String(s) => Some(format!("{key}={}", encode_query_value(s))),
					_ => {
						warn!(
							"Query parameter '{}' for tool '{}' unsupported (value: {:?}), skipping",
							param_name, name, v
						);
						None
					},
				}
			};
			query_params
				.iter()
				.flat_map(|(k, v)| {
					let key = encode_query_value(k);
					match v {
						Value::Array(a) => a
							.iter()
							.filter_map(|v| format_pair(k, &key, v))
							.collect::<Vec<_>>(),
						_ => format_pair(k, &key, v).into_iter().collect(),
					}
				})
				.fold(String::new(), |mut acc, pair| {
					acc.push(if acc.is_empty() { '?' } else { '&' });
					acc.push_str(&pair);
					acc
				})
		};

		let uri = format!("{base_url}{query_string}");

		let mut rb = http::Request::builder()
			.method(method)
			.uri(uri)
			.header(ACCEPT, HeaderValue::from_static("application/json"));

		// Build request body
		let body = if let Some(body_val) = body_value {
			rb = rb.header(CONTENT_TYPE, HeaderValue::from_static("application/json"));
			serde_json::to_vec(&body_val).map_err(|e| UpstreamError::OpenAPIError(e.into()))?
		} else {
			Vec::new()
		};

		// Build the final request
		let mut request = rb
			.body(body.into())
			.map_err(|e| anyhow::anyhow!("Failed to build request: {}", e))?;

		ctx.apply(&mut request);

		// First header set wins including headers set by ctx.apply or the gateway
		for (key, value) in &header_params {
			// Only allow headers defined in the OpenAPI schema
			if !info.allowed_headers.contains(key) {
				debug!(
					"Ignoring header '{}' for tool '{}' not defined in schema",
					key, name
				);
				continue;
			}

			if let Some(s_val) = value.as_str() {
				match (
					HeaderName::from_bytes(key.as_bytes()),
					HeaderValue::from_str(s_val),
				) {
					(Ok(header_name), Ok(header_value)) => {
						// Ingore if header is protected
						if header_name == CONTENT_LENGTH
							|| header_name == CONTENT_TYPE
							|| header_name == TRANSFER_ENCODING
							|| header_name == HOST
						{
							debug!("Ignoring protected header '{}' for tool '{}'", key, name);
							continue;
						}

						// Don't override existing headers
						if request.headers().contains_key(&header_name) {
							debug!("Ingoring header '{}' for tool '{}' already set", key, name);
							continue;
						}

						request.headers_mut().insert(header_name, header_value);
					},
					(Err(_), _) => warn!(
						"Invalid header name '{}' for tool '{}', skipping",
						key, name
					),
					(_, Err(_)) => warn!(
						"Invalid header value '{}' for header '{}' in tool '{}', skipping",
						s_val, key, name
					),
				}
			} else {
				warn!(
					"Header parameter '{}' for tool '{}' is not a string (value: {:?}), skipping",
					key, name, value
				);
			}
		}

		let response = self.http_client.call(request).await?;

		// Read response body
		let status = response.status();
		// Check if the request was successful
		if status.is_success() {
			let lim = crate::http::response_buffer_limit(&response);
			let content_encoding = response.headers().typed_get::<headers::ContentEncoding>();
			let body_bytes = crate::http::compression::to_bytes_with_decompression(
				response.into_body(),
				content_encoding.as_ref(),
				lim,
			)
			.await
			.map_err(|e| UpstreamError::OpenAPIError(e.into()))?
			.1;

			// Wrap responses that are not structuredContent compliant in object
			Ok(json!({ "data":
				match serde_json::from_slice::<serde_json::Value>(&body_bytes).map_err(|e| UpstreamError::OpenAPIError(e.into()))? {
					Value::Object(obj) => return Ok(Value::Object(obj)),
					Value::Null => return Ok(Value::Null),
					data => data,
			}}))
		} else {
			let lim = crate::http::response_buffer_limit(&response);
			let body = String::from_utf8(
				crate::http::read_body_with_limit(response.into_body(), lim)
					.await
					.map_err(|e| UpstreamError::OpenAPIError(e.into()))?
					.to_vec(),
			)
			.map_err(|e| UpstreamError::OpenAPIError(e.into()))?;
			Err(UpstreamError::OpenAPIError(anyhow::anyhow!(
				"Upstream API call for tool '{}' failed with status {}: {}",
				name,
				status,
				body
			)))
		}
	}

	pub fn tools(&self) -> Vec<Tool> {
		self.tools.clone().into_iter().map(|(t, _)| t).collect()
	}
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
