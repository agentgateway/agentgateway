// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use agent_core::drain::DrainWatcher;
use headers::Header;
use headers_accept::Accept;
use hyper::Request;
use hyper::body::Incoming;
use mediatype::MediaType;
use prometheus_client::encoding::protobuf::encode as encode_protobuf;
use prometheus_client::encoding::text::encode as encode_text;
use prometheus_client::registry::Registry;
use prost_v12::Message;

use super::hyper_helpers;
use crate::Address;
use crate::http::Response;

pub struct Server {
	s: hyper_helpers::Server<Mutex<Registry>>,
}

impl Server {
	pub async fn new(
		addr: Address,
		drain_rx: DrainWatcher,
		registry: Registry,
	) -> anyhow::Result<Self> {
		hyper_helpers::Server::<Mutex<Registry>>::bind("stats", addr, drain_rx, Mutex::new(registry))
			.await
			.map(|s| Server { s })
	}

	pub fn address(&self) -> SocketAddr {
		self.s.address()
	}

	pub fn spawn(self) {
		self.s.spawn(|registry, req| async move {
			match req.uri().path() {
				"/metrics" | "/stats/prometheus" => Ok(handle_metrics(registry, req).await),
				_ => Ok(hyper_helpers::empty_response(hyper::StatusCode::NOT_FOUND)),
			}
		})
	}
}

async fn handle_metrics(reg: Arc<Mutex<Registry>>, req: Request<Incoming>) -> Response {
	let reg = reg.lock().expect("mutex");
	let content_type = content_type(&req);
	let result = match content_type {
		ContentType::PlainText | ContentType::OpenMetrics => {
			let mut str_buf = String::new();
			encode_text(&mut str_buf, &reg).map(|_| str_buf.into_bytes())
		},
		ContentType::Protobuf => {
			encode_protobuf(&reg).map(|metrics| metrics.encode_length_delimited_to_vec())
		},
	};
	match result {
		Ok(buf) => ::http::Response::builder()
			.status(hyper::StatusCode::OK)
			.header(
				hyper::header::CONTENT_TYPE,
				Into::<&str>::into(content_type),
			)
			.body(buf.into()),
		Err(err) => ::http::Response::builder()
			.status(hyper::StatusCode::INTERNAL_SERVER_ERROR)
			.body(err.to_string().into()),
	}
	.expect("builder with known status code should not fail")
}

#[derive(Default)]
enum ContentType {
	#[default]
	PlainText,
	OpenMetrics,
	Protobuf,
}

impl From<ContentType> for &str {
	fn from(c: ContentType) -> Self {
		match c {
			ContentType::PlainText => "text/plain;charset=utf-8",
			ContentType::OpenMetrics => "application/openmetrics-text;charset=utf-8;version=1.0.0",
			ContentType::Protobuf => {
				"application/vnd.google.protobuf;proto=io.prometheus.client.MetricSet;encoding=delimited;version=1.0.0"
			},
		}
	}
}

fn content_type_from_media_type(m: MediaType) -> Option<ContentType> {
	let ty_str: &str = m.ty.as_str();
	if ty_str == mediatype::names::TEXT.as_str() && m.subty == mediatype::names::PLAIN.as_str() {
		return Some(ContentType::PlainText);
	} else if ty_str != mediatype::names::APPLICATION.as_str() {
		return None;
	}
	match m.subty.as_str() {
		"openmetrics-text" => Some(ContentType::OpenMetrics),
		"vnd.google.protobuf" | "protobuf" | "x-protobuf" => Some(ContentType::Protobuf),
		_ => None,
	}
}

const AVAILABLE_MEDIA_TYPES: [MediaType<'static>; 5] = [
	MediaType::new(
		mediatype::names::APPLICATION,
		mediatype::Name::new_unchecked("vnd.google.protobuf"),
	),
	MediaType::new(
		mediatype::names::APPLICATION,
		mediatype::Name::new_unchecked("protobuf"),
	),
	MediaType::new(
		mediatype::names::APPLICATION,
		mediatype::Name::new_unchecked("x-protobuf"),
	),
	MediaType::new(
		mediatype::names::APPLICATION,
		mediatype::Name::new_unchecked("openmetrics-text"),
	),
	MediaType::new(mediatype::names::TEXT, mediatype::names::PLAIN),
];

#[inline(always)]
fn content_type<T>(req: &Request<T>) -> ContentType {
	let mut values = req.headers().get_all(http::header::ACCEPT).iter();
	let accept = match Accept::decode(&mut values) {
		Ok(header) => header,
		Err(_) => return ContentType::default(),
	};
	accept
		// Using this call ensures quality parameters are handled correctly.
		// We don't use Accept::negotiate, because extra parameters are not
		// enforced and create mismatch conditions that require creating more
		// mappings in AVAILABLE_MEDIA_TYPES for every case.
		.media_types()
		.map(mediatype::MediaTypeBuf::essence)
		.find(|mediatype| {
			AVAILABLE_MEDIA_TYPES
				.iter()
				.any(|available| mediatype == available)
		})
		.and_then(content_type_from_media_type)
		.unwrap_or_default()
}

mod test {
	#[test]
	fn test_content_type() {
		let plain_text_req = http::Request::new("I want some plain text");
		assert_eq!(
			Into::<&str>::into(super::content_type(&plain_text_req)),
			"text/plain;charset=utf-8"
		);

		let openmetrics_req = http::Request::builder()
			.header("X-Custom-Beep", "boop")
			.header("Accept", "application/json")
			.header("Accept", "application/openmetrics-text; other stuff")
			.body("Invalid header defaulting to text/plain")
			.unwrap();
		assert_eq!(
			Into::<&str>::into(super::content_type(&openmetrics_req)),
			"text/plain;charset=utf-8"
		);

		let openmetrics_req = http::Request::builder()
			.header("X-Custom-Beep", "boop")
			.header("Accept", "application/json")
			.header("Accept", "application/openmetrics-text;version=1.0.0")
			.body("I would like openmetrics")
			.unwrap();
		assert_eq!(
			Into::<&str>::into(super::content_type(&openmetrics_req)),
			"application/openmetrics-text;charset=utf-8;version=1.0.0"
		);

		let mixed_req = http::Request::builder()
          .header("X-Custom-Beep", "boop")
          .header("Accept", "application/vnd.google.protobuf;proto=io.prometheus.client.MetricSet;encoding=delimited;q=0.6,application/openmetrics-text;version=1.0.0;escaping=allow-utf-8;q=0.5,application/openmetrics-text;version=0.0.1;q=0.4,text/plain;version=1.0.0;escaping=allow-utf-8;q=0.3,text/plain;version=0.0.4;q=0.2,*/*;q=0.1")
          .body("I would like protobuf")
          .unwrap();
		assert_eq!(
			Into::<&str>::into(super::content_type(&mixed_req)),
			"application/vnd.google.protobuf;proto=io.prometheus.client.MetricSet;encoding=delimited;version=1.0.0"
		);

		let unsupported_req_accept = http::Request::builder()
			.header("Accept", "application/json")
			.body("I would like some json")
			.unwrap();
		// asking for something we don't support, fall back to plaintext
		assert_eq!(
			Into::<&str>::into(super::content_type(&unsupported_req_accept)),
			"text/plain;charset=utf-8"
		)
	}
}
