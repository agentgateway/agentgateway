use std::sync::Arc;

use cel::objects::ListValue;
use cel::types::dynamic::{DynamicType, DynamicValue};
use cel::{ExecutionError, FunctionContext, Value};
use serde::{Serialize, Serializer};
use url::form_urlencoded;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryAccessorKind {
	PathAndQuery,
	Uri,
}

#[derive(Debug, Clone)]
pub struct QueryAccessor<'a> {
	kind: QueryAccessorKind,
	uri: &'a http::Uri,
}

#[derive(Debug, Clone)]
struct OwnedQueryAccessor {
	kind: QueryAccessorKind,
	uri: http::Uri,
}

trait QueryAccessorOps {
	fn kind(&self) -> QueryAccessorKind;
	fn uri(&self) -> &http::Uri;

	fn query_str(&self) -> Option<&str> {
		self.uri().query()
	}

	fn with_updated_query(&self, key: &str, value: &str, set: bool) -> OwnedQueryAccessor {
		let query = updated_query(self.query_str(), key, value, set);
		OwnedQueryAccessor {
			kind: self.kind(),
			uri: build_uri(self.kind(), self.uri(), &query),
		}
	}
}

impl<'a> QueryAccessor<'a> {
	pub fn path_and_query_from_uri(uri: &'a http::Uri) -> Self {
		Self {
			kind: QueryAccessorKind::PathAndQuery,
			uri,
		}
	}

	pub fn uri_from_uri(uri: &'a http::Uri) -> Self {
		Self {
			kind: QueryAccessorKind::Uri,
			uri,
		}
	}

	fn display(&self) -> &str {
		match self.kind {
			QueryAccessorKind::PathAndQuery => self
				.uri
				.path_and_query()
				.map(|s| s.as_str())
				.unwrap_or_else(|| self.uri.path()),
			QueryAccessorKind::Uri => unreachable!("uri display is not borrowed"),
		}
	}
}

impl QueryAccessorOps for QueryAccessor<'_> {
	fn kind(&self) -> QueryAccessorKind {
		self.kind
	}

	fn uri(&self) -> &http::Uri {
		self.uri
	}
}

impl OwnedQueryAccessor {
	fn display(&self) -> &str {
		match self.kind {
			QueryAccessorKind::PathAndQuery => self
				.uri
				.path_and_query()
				.map(|s| s.as_str())
				.unwrap_or_else(|| self.uri.path()),
			QueryAccessorKind::Uri => unreachable!("uri display is not borrowed"),
		}
	}
}

impl QueryAccessorOps for OwnedQueryAccessor {
	fn kind(&self) -> QueryAccessorKind {
		self.kind
	}

	fn uri(&self) -> &http::Uri {
		&self.uri
	}
}

fn call_query<'a, 'rf, T: QueryAccessorOps>(
	this: &T,
	ftx: &mut FunctionContext<'a, 'rf>,
) -> cel::ResolveResult<'a> {
	if ftx.args.len() != 1 {
		return Err(ExecutionError::invalid_argument_count(1, ftx.args.len()));
	}
	let key = ftx.arg::<Arc<str>>(0)?;
	let Some(query) = this.query_str() else {
		return Err(ExecutionError::no_such_key(key.as_ref()));
	};
	let values = form_urlencoded::parse(query.as_bytes())
		.filter(|(k, _)| k == key.as_ref())
		.map(|(_, value)| Value::from(value.into_owned()))
		.collect::<Vec<_>>();
	if values.is_empty() {
		return Err(ExecutionError::no_such_key(key.as_ref()));
	}
	Ok(Value::List(ListValue::PartiallyOwned(values.into())))
}

fn call_update<'a, 'rf, T: QueryAccessorOps>(
	this: &T,
	set: bool,
	ftx: &mut FunctionContext<'a, 'rf>,
) -> cel::ResolveResult<'a> {
	if ftx.args.len() != 2 {
		return Err(ExecutionError::invalid_argument_count(2, ftx.args.len()));
	}
	let key = ftx.arg::<Arc<str>>(0)?;
	let value = ftx.arg::<Arc<str>>(1)?;
	let updated = this.with_updated_query(key.as_ref(), value.as_ref(), set);
	Ok(Value::Dynamic(DynamicValue::new_owned(updated)))
}

fn call_function_impl<'a, 'rf, T: QueryAccessorOps>(
	this: &T,
	name: &str,
	ftx: &mut FunctionContext<'a, 'rf>,
) -> Option<cel::ResolveResult<'a>> {
	match name {
		"query" => Some(call_query(this, ftx)),
		"addQuery" => Some(call_update(this, false, ftx)),
		"setQuery" => Some(call_update(this, true, ftx)),
		_ => None,
	}
}

fn updated_query(current: Option<&str>, key: &str, value: &str, set: bool) -> String {
	let mut pairs = match current {
		Some(query) => form_urlencoded::parse(query.as_bytes())
			.map(|(k, v)| (k.into_owned(), v.into_owned()))
			.collect::<Vec<_>>(),
		None => Vec::new(),
	};
	if set {
		pairs.retain(|(existing, _)| existing != key);
	}
	pairs.push((key.to_string(), value.to_string()));

	let mut serializer = form_urlencoded::Serializer::new(String::new());
	for (key, value) in pairs {
		serializer.append_pair(&key, &value);
	}
	serializer.finish()
}

fn build_uri(kind: QueryAccessorKind, base: &http::Uri, query: &str) -> http::Uri {
	let path_and_query = if query.is_empty() {
		base.path().parse()
	} else {
		format!("{}?{query}", base.path()).parse()
	}
	.expect("existing HTTP path and rebuilt query must stay valid");

	match kind {
		QueryAccessorKind::PathAndQuery => {
			let mut parts = http::uri::Parts::default();
			parts.path_and_query = Some(path_and_query);
			http::Uri::from_parts(parts).expect("relative path-and-query must stay valid")
		},
		QueryAccessorKind::Uri => {
			let mut parts = http::uri::Parts::default();
			parts.scheme = base.scheme().cloned();
			parts.authority = base.authority().cloned();
			parts.path_and_query = Some(path_and_query);
			http::Uri::from_parts(parts).expect("absolute URI components must stay valid")
		},
	}
}

impl Serialize for QueryAccessor<'_> {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self.kind {
			QueryAccessorKind::PathAndQuery => serializer.serialize_str(self.display()),
			QueryAccessorKind::Uri => serializer.collect_str(self.uri),
		}
	}
}

impl DynamicType for QueryAccessor<'_> {
	fn materialize(&self) -> Value<'_> {
		match self.kind {
			QueryAccessorKind::PathAndQuery => Value::from(self.display()),
			QueryAccessorKind::Uri => Value::from(self.uri.to_string()),
		}
	}

	fn call_function<'a, 'rf>(
		&self,
		name: &str,
		ftx: &mut FunctionContext<'a, 'rf>,
	) -> Option<cel::ResolveResult<'a>>
	where
		Self: 'a,
	{
		call_function_impl(self, name, ftx)
	}
}

impl Serialize for OwnedQueryAccessor {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self.kind {
			QueryAccessorKind::PathAndQuery => serializer.serialize_str(self.display()),
			QueryAccessorKind::Uri => serializer.collect_str(&self.uri),
		}
	}
}

impl DynamicType for OwnedQueryAccessor {
	fn materialize(&self) -> Value<'_> {
		match self.kind {
			QueryAccessorKind::PathAndQuery => Value::from(self.display()),
			QueryAccessorKind::Uri => Value::from(self.uri.to_string()),
		}
	}

	fn call_function<'a, 'rf>(
		&self,
		name: &str,
		ftx: &mut FunctionContext<'a, 'rf>,
	) -> Option<cel::ResolveResult<'a>>
	where
		Self: 'a,
	{
		call_function_impl(self, name, ftx)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn add_query() {
		assert_eq!(
			updated_query(Some("foo=bar&foo=baz&zap=zip"), "foo", "qux", false),
			"foo=bar&foo=baz&zap=zip&foo=qux"
		);
	}

	#[test]
	fn set_query() {
		assert_eq!(
			updated_query(Some("foo=bar&foo=baz&zap=zip"), "foo", "qux", true),
			"zap=zip&foo=qux"
		);
	}

	#[test]
	fn build_uri_relative() {
		let base = "/api/test?foo=bar&zap=zip".parse::<http::Uri>().unwrap();
		let built = build_uri(QueryAccessorKind::PathAndQuery, &base, "zap=zip&foo=qux");
		assert_eq!(built.to_string(), "/api/test?zap=zip&foo=qux");
		assert!(built.scheme().is_none());
		assert!(built.authority().is_none());
	}

	#[test]
	fn build_uri_absolute() {
		let base = "http://example.com/api/test?foo=bar&zap=zip"
			.parse::<http::Uri>()
			.unwrap();
		let built = build_uri(QueryAccessorKind::Uri, &base, "zap=zip&foo=qux");
		assert_eq!(
			built.to_string(),
			"http://example.com/api/test?zap=zip&foo=qux"
		);
	}

	#[test]
	fn multiple_updates() {
		let base = "http://example.com/api/test?foo=bar"
			.parse::<http::Uri>()
			.unwrap();
		let accessor = QueryAccessor::uri_from_uri(&base);
		let updated = accessor.with_updated_query("foo", "baz", false);
		let updated = updated.with_updated_query("foo", "qux", true);
		let updated = updated.with_updated_query("a", "b", true);
		assert_eq!(
			updated.uri.to_string(),
			"http://example.com/api/test?foo=qux&a=b"
		);
	}
}
