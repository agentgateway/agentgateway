use std::str::FromStr;

use ::http::{HeaderValue, Method, StatusCode, header};
use duration_str::HumanFormat;
use serde::de::Error;
use serde::ser::SerializeMap;

use crate::http::{Request, Response, filters};
use crate::types::agent::{HostRedirect, PathRedirect};
use crate::*;

#[derive(Default, Debug, Clone)]
enum WildcardOrList<T> {
	#[default]
	None,
	Wildcard,
	List(Vec<T>),
}

impl<T> WildcardOrList<T> {
	fn is_none(&self) -> bool {
		matches!(self, WildcardOrList::None)
	}
}

impl<T: FromStr> TryFrom<Vec<String>> for WildcardOrList<T> {
	type Error = T::Err;

	fn try_from(value: Vec<String>) -> Result<Self, Self::Error> {
		if value.contains(&"*".to_string()) {
			Ok(WildcardOrList::Wildcard)
		} else if value.is_empty() {
			Ok(WildcardOrList::None)
		} else {
			let vec: Vec<T> = value
				.into_iter()
				.map(|v| T::from_str(&v))
				.collect::<Result<_, _>>()?;
			Ok(WildcardOrList::List(vec))
		}
	}
}

impl<T: Display> Serialize for WildcardOrList<T> {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		match self {
			WildcardOrList::None => Vec::<String>::new().serialize(serializer),
			WildcardOrList::Wildcard => vec!["*"].serialize(serializer),
			WildcardOrList::List(list) => list
				.iter()
				.map(ToString::to_string)
				.collect::<Vec<_>>()
				.serialize(serializer),
		}
	}
}

impl<T> WildcardOrList<T>
where
	T: ToString,
{
	fn to_header_value(&self) -> Option<::http::HeaderValue> {
		match self {
			WildcardOrList::None => None,
			WildcardOrList::Wildcard => Some(::http::HeaderValue::from_static("*")),
			WildcardOrList::List(list) => {
				let value = list
					.iter()
					.map(|item| item.to_string())
					.collect::<Vec<_>>()
					.join(",");

				::http::HeaderValue::from_str(&value).ok()
			},
		}
	}
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[cfg_attr(feature = "schema", schemars(with = "CorsSerde"))]
pub struct Cors {
	allow_credentials: bool,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	allow_headers: WildcardOrList<http::HeaderName>,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	allow_methods: WildcardOrList<::http::Method>,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	allow_origins: WildcardOrList<Strng>,
	#[serde(skip_serializing_if = "WildcardOrList::is_none")]
	expose_headers: WildcardOrList<http::HeaderName>,
	#[serde(serialize_with = "ser_string_or_bytes_option")]
	max_age: Option<::http::HeaderValue>,
}

impl<'de> serde::Deserialize<'de> for Cors {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		Cors::try_from(CorsSerde::deserialize(deserializer)?).map_err(D::Error::custom)
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct CorsSerde {
	#[serde(default)]
	pub allow_credentials: bool,
	#[serde(default)]
	pub allow_headers: Vec<String>,
	#[serde(default)]
	pub allow_methods: Vec<String>,
	#[serde(default)]
	pub allow_origins: Vec<String>,
	#[serde(default)]
	pub expose_headers: Vec<String>,
	#[serde(default, with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pub max_age: Option<Duration>,
}

impl TryFrom<CorsSerde> for Cors {
	type Error = anyhow::Error;
	fn try_from(value: CorsSerde) -> Result<Self, Self::Error> {
		Ok(Cors {
			allow_credentials: value.allow_credentials,
			allow_headers: WildcardOrList::try_from(value.allow_headers)?,
			allow_methods: WildcardOrList::try_from(value.allow_methods)?,
			allow_origins: WildcardOrList::try_from(value.allow_origins)?,
			expose_headers: WildcardOrList::try_from(value.expose_headers)?,
			max_age: value
				.max_age
				.map(|v| http::HeaderValue::from_str(&v.as_secs().to_string()))
				.transpose()?,
		})
	}
}

impl Cors {
	/// Apply applies the CORS header. It seems a lot of implementations handle this differently wrt when
	/// to add or not add headers, and when to forward the request.
	/// We follow Envoy semantics here (with forwardNotMatchingPreflights=true)
	pub fn apply(&self, req: &mut Request) -> Result<CorsResponse, filters::Error> {
		// If no origin, return immediately
		let Some(origin) = req.headers().get(header::ORIGIN) else {
			return Ok(Default::default());
		};

		let allowed = match &self.allow_origins {
			WildcardOrList::None => false,
			WildcardOrList::Wildcard => true,
			WildcardOrList::List(origins) => {
				// TODO: allow wildcards
				let os = origin.as_bytes();
				origins.iter().any(|want| want.as_bytes() == os)
			},
		};
		if !allowed {
			// None matching origin, return
			return Ok(Default::default());
		}

		if req.method() == Method::OPTIONS {
			// Handle preflight request
			let mut rb = ::http::Response::builder()
				.status(StatusCode::OK)
				.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
			if let Some(h) = self.allow_methods.to_header_value() {
				rb = rb.header(header::ACCESS_CONTROL_ALLOW_METHODS, h);
			}
			if let Some(h) = self.allow_headers.to_header_value() {
				rb = rb.header(header::ACCESS_CONTROL_ALLOW_HEADERS, h);
			}
			if let Some(h) = &self.max_age {
				rb = rb.header(header::ACCESS_CONTROL_MAX_AGE, h);
			}
			let response = rb.body(crate::http::Body::empty())?;
			return Ok(CorsResponse {
				direct_response: Some(response),
				response_headers: None,
			});
		}

		let mut response_headers = http::HeaderMap::with_capacity(3);
		response_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
		if self.allow_credentials {
			response_headers.insert(header::ACCESS_CONTROL_ALLOW_CREDENTIALS, HEADER_VALUE_TRUE);
		}
		if let Some(h) = self.expose_headers.to_header_value() {
			response_headers.insert(header::ACCESS_CONTROL_EXPOSE_HEADERS, h);
		}
		// For actual requests, we would need to add CORS headers to the response
		// but since we only have access to the request here, we return None
		Ok(CorsResponse {
			direct_response: None,
			response_headers: Some(response_headers),
		})
	}
}

const HEADER_VALUE_TRUE: http::HeaderValue = HeaderValue::from_static("true");

#[derive(Debug, Default)]
pub struct CorsResponse {
	pub direct_response: Option<Response>,
	pub response_headers: Option<http::HeaderMap>,
}
