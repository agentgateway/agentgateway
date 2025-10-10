use std::convert::TryFrom;
use std::fmt;

use http::header::{Entry, OccupiedEntry};
use http::request::Parts;
use http::{Extensions, Request as HttpRequest, Version};
use hyper_util::client::legacy::connect::Connect;
use reqwest::IntoUrl;
use serde::Serialize;
use serde_json;
use url::Url;

use crate::http::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use crate::http::{Body, Method, Response};

/// A request which can be executed with `Client::execute()`.
pub struct Request {
	method: Method,
	url: Url,
	headers: HeaderMap,
	body: Option<Body>,
	version: Version,
	extensions: Extensions,
	path_match: Option<String>,
}

/// A builder to construct the properties of a `Request`.
///
/// To construct a `RequestBuilder`, refer to the `Client` documentation.
#[must_use = "RequestBuilder does nothing until you 'send' it"]
pub struct RequestBuilder {
	request: Result<Request, crate::http::Error>,
}

impl Request {
	/// Constructs a new request.
	#[inline]
	pub fn new(method: Method, url: Url) -> Self {
		Request {
			method,
			url,
			headers: HeaderMap::new(),
			body: None,
			version: Version::default(),
			extensions: Extensions::new(),
			path_match: None,
		}
	}

	/// Get the method.
	#[inline]
	pub fn method(&self) -> &Method {
		&self.method
	}

	/// Get a mutable reference to the method.
	#[inline]
	pub fn method_mut(&mut self) -> &mut Method {
		&mut self.method
	}

	/// Get the url.
	#[inline]
	pub fn url(&self) -> &Url {
		&self.url
	}

	/// Get a mutable reference to the url.
	#[inline]
	pub fn url_mut(&mut self) -> &mut Url {
		&mut self.url
	}

	/// Get the headers.
	#[inline]
	pub fn headers(&self) -> &HeaderMap {
		&self.headers
	}

	/// Get a mutable reference to the headers.
	#[inline]
	pub fn headers_mut(&mut self) -> &mut HeaderMap {
		&mut self.headers
	}

	/// Get the body.
	#[inline]
	pub fn body(&self) -> Option<&Body> {
		self.body.as_ref()
	}

	/// Get a mutable reference to the body.
	#[inline]
	pub fn body_mut(&mut self) -> &mut Option<Body> {
		&mut self.body
	}

	/// Get the http version.
	#[inline]
	pub fn version(&self) -> Version {
		self.version
	}

	/// Get a mutable reference to the http version.
	#[inline]
	pub fn version_mut(&mut self) -> &mut Version {
		&mut self.version
	}
}

impl RequestBuilder {
	pub fn new<U: IntoUrl>(method: Method, url: U) -> Self {
		RequestBuilder {
			request: url
				.into_url()
				.map(|u| Request::new(method, u))
				.map_err(crate::http::Error::new),
		}
	}

	/// Add a `Header` to this Request.
	pub fn header<K, V>(self, key: K, value: V) -> RequestBuilder
	where
		HeaderName: TryFrom<K>,
		<HeaderName as TryFrom<K>>::Error: Into<http::Error>,
		HeaderValue: TryFrom<V>,
		<HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
	{
		self.header_sensitive(key, value, false)
	}

	/// Add a `Header` to this Request with ability to define if `header_value` is sensitive.
	fn header_sensitive<K, V>(mut self, key: K, value: V, sensitive: bool) -> RequestBuilder
	where
		HeaderName: TryFrom<K>,
		<HeaderName as TryFrom<K>>::Error: Into<http::Error>,
		HeaderValue: TryFrom<V>,
		<HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
	{
		let mut error = None;
		if let Ok(ref mut req) = self.request {
			match <HeaderName as TryFrom<K>>::try_from(key) {
				Ok(key) => match <HeaderValue as TryFrom<V>>::try_from(value) {
					Ok(mut value) => {
						// We want to potentially make an non-sensitive header
						// to be sensitive, not the reverse. So, don't turn off
						// a previously sensitive header.
						if sensitive {
							value.set_sensitive(true);
						}
						req.headers_mut().append(key, value);
					},
					Err(e) => error = Some(crate::http::Error::new(e.into())),
				},
				Err(e) => error = Some(crate::http::Error::new(e.into())),
			};
		}
		if let Some(err) = error {
			self.request = Err(err);
		}
		self
	}

	/// Add a set of Headers to the existing ones on this Request.
	///
	/// The headers will be merged in to any already set.
	pub fn headers(mut self, headers: http::header::HeaderMap) -> RequestBuilder {
		if let Ok(ref mut req) = self.request {
			replace_headers(req.headers_mut(), headers);
		}
		self
	}

	/// Set the request body.
	pub fn body<T: Into<Body>>(mut self, body: T) -> RequestBuilder {
		if let Ok(ref mut req) = self.request {
			*req.body_mut() = Some(body.into());
		}
		self
	}

	/// Modify the query string of the URL.
	///
	/// Modifies the URL of this request, adding the parameters provided.
	/// This method appends and does not overwrite. This means that it can
	/// be called multiple times and that existing query parameters are not
	/// overwritten if the same key is used. The key will simply show up
	/// twice in the query string.
	/// Calling `.query(&[("foo", "a"), ("foo", "b")])` gives `"foo=a&foo=b"`.
	///
	/// # Note
	/// This method does not support serializing a single key-value
	/// pair. Instead of using `.query(("key", "val"))`, use a sequence, such
	/// as `.query(&[("key", "val")])`. It's also possible to serialize structs
	/// and maps into a key-value pair.
	///
	/// # Errors
	/// This method will fail if the object you provide cannot be serialized
	/// into a query string.
	pub fn query<T: Serialize + ?Sized>(mut self, query: &T) -> RequestBuilder {
		let mut error = None;
		if let Ok(ref mut req) = self.request {
			let url = req.url_mut();
			let mut pairs = url.query_pairs_mut();
			let serializer = serde_urlencoded::Serializer::new(&mut pairs);

			if let Err(err) = query.serialize(serializer) {
				error = Some(crate::http::Error::new(err));
			}
		}
		if let Ok(ref mut req) = self.request
			&& let Some("") = req.url().query()
		{
			req.url_mut().set_query(None);
		}
		if let Some(err) = error {
			self.request = Err(err);
		}
		self
	}

	/// Set HTTP version
	pub fn version(mut self, version: Version) -> RequestBuilder {
		if let Ok(ref mut req) = self.request {
			req.version = version;
		}
		self
	}

	/// Send a form body.
	///
	/// Sets the body to the url encoded serialization of the passed value,
	/// and also sets the `Content-Type: application/x-www-form-urlencoded`
	/// header.
	///
	/// ```rust
	/// # use reqwest::Error;
	/// # use std::collections::HashMap;
	/// #
	/// # async fn run() -> Result<(), Error> {
	/// let mut params = HashMap::new();
	/// params.insert("lang", "rust");
	///
	/// let client = reqwest::Client::new();
	/// let res = client.post("http://httpbin.org")
	///     .form(&params)
	///     .send()
	///     .await?;
	/// # Ok(())
	/// # }
	/// ```
	///
	/// # Errors
	///
	/// This method fails if the passed value cannot be serialized into
	/// url encoded format
	pub fn form<T: Serialize + ?Sized>(mut self, form: &T) -> RequestBuilder {
		let mut error = None;
		if let Ok(ref mut req) = self.request {
			match serde_urlencoded::to_string(form) {
				Ok(body) => {
					req
						.headers_mut()
						.entry(CONTENT_TYPE)
						.or_insert(HeaderValue::from_static(
							"application/x-www-form-urlencoded",
						));
					*req.body_mut() = Some(body.into());
				},
				Err(err) => error = Some(crate::http::Error::new(err)),
			}
		}
		if let Some(err) = error {
			self.request = Err(err);
		}
		self
	}

	/// Send a JSON body.
	///
	/// # Optional
	///
	/// This requires the optional `json` feature enabled.
	///
	/// # Errors
	///
	/// Serialization can fail if `T`'s implementation of `Serialize` decides to
	/// fail, or if `T` contains a map with non-string keys.
	pub fn json<T: Serialize + ?Sized>(mut self, json: &T) -> RequestBuilder {
		let mut error = None;
		if let Ok(ref mut req) = self.request {
			match serde_json::to_vec(json) {
				Ok(body) => {
					if !req.headers().contains_key(CONTENT_TYPE) {
						req
							.headers_mut()
							.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
					}
					*req.body_mut() = Some(body.into());
				},
				Err(err) => error = Some(crate::http::Error::new(err)),
			}
		}
		if let Some(err) = error {
			self.request = Err(err);
		}
		self
	}

	// This was a shell only meant to help with rendered documentation.
	// However, docs.rs can now show the docs for the wasm platforms, so this
	// is no longer needed.
	//
	// You should not otherwise depend on this function. It's deprecation
	// is just to nudge people to reduce breakage. It may be removed in a
	// future patch version.
	#[doc(hidden)]
	#[cfg_attr(target_arch = "wasm32", deprecated)]
	pub fn fetch_mode_no_cors(self) -> RequestBuilder {
		self
	}

	pub fn path_match(mut self, path_match: String) -> Self {
		if let Ok(ref mut req) = self.request {
			req.path_match = Some(path_match);
		}
		self
	}

	pub fn build(self) -> Result<crate::http::Request, crate::http::Error> {
		let req = crate::http::Request::try_from(self.request?)?;
		Ok(req)
	}

	pub async fn send<C>(
		self,
		client: hyper_util::client::legacy::Client<C, crate::http::Body>,
	) -> Result<Response, crate::http::Error>
	where
		C: Connect + Clone + Send + Sync + 'static,
	{
		let req = crate::http::Request::try_from(self.request?)?;
		Ok(
			client
				.request(req)
				.await
				.map_err(crate::http::Error::new)?
				.map(crate::http::Body::new),
		)
	}
}

impl fmt::Debug for Request {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		fmt_request_fields(&mut f.debug_struct("Request"), self).finish()
	}
}

impl fmt::Debug for RequestBuilder {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		let mut builder = f.debug_struct("RequestBuilder");
		match self.request {
			Ok(ref req) => fmt_request_fields(&mut builder, req).finish(),
			Err(ref err) => builder.field("error", err).finish(),
		}
	}
}

fn fmt_request_fields<'a, 'b>(
	f: &'a mut fmt::DebugStruct<'a, 'b>,
	req: &Request,
) -> &'a mut fmt::DebugStruct<'a, 'b> {
	f.field("method", &req.method)
		.field("url", &req.url)
		.field("headers", &req.headers)
}

impl TryFrom<crate::http::Request> for Request {
	type Error = crate::http::Error;

	fn try_from(req: crate::http::Request) -> Result<Self, crate::http::Error> {
		let (parts, body) = req.into_parts();
		let Parts {
			method,
			uri,
			headers,
			version,
			extensions,
			..
		} = parts;
		let url = Url::parse(&uri.to_string()).map_err(crate::http::Error::new)?;
		Ok(Request {
			method,
			url,
			headers,
			body: Some(body),
			version,
			extensions,
			path_match: None,
		})
	}
}

impl TryFrom<Request> for crate::http::Request {
	type Error = crate::http::Error;

	fn try_from(req: Request) -> Result<Self, crate::http::Error> {
		let Request {
			method,
			url,
			headers,
			body,
			version,
			extensions,
			..
		} = req;

		let mut req = HttpRequest::builder()
			.version(version)
			.method(method)
			.uri(url.as_str())
			.body(body.unwrap_or_else(Body::empty))
			.map_err(crate::http::Error::new)?;

		*req.headers_mut() = headers;
		*req.extensions_mut() = extensions;
		Ok(req)
	}
}

pub(crate) fn replace_headers(dst: &mut HeaderMap, src: HeaderMap) {
	// IntoIter of HeaderMap yields (Option<HeaderName>, HeaderValue).
	// The first time a name is yielded, it will be Some(name), and if
	// there are more values with the same name, the next yield will be
	// None.

	let mut prev_entry: Option<OccupiedEntry<_>> = None;
	for (key, value) in src {
		match key {
			Some(key) => match dst.entry(key) {
				Entry::Occupied(mut e) => {
					e.insert(value);
					prev_entry = Some(e);
				},
				Entry::Vacant(e) => {
					let e = e.insert_entry(value);
					prev_entry = Some(e);
				},
			},
			None => match prev_entry {
				Some(ref mut entry) => {
					entry.append(value);
				},
				None => unreachable!("HeaderMap::into_iter yielded None first"),
			},
		}
	}
}
