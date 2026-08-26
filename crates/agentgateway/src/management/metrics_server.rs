// Originally derived from https://github.com/istio/ztunnel (Apache 2.0 licensed)

use std::net::SocketAddr;
use std::sync::Arc;

use agent_core::drain::DrainWatcher;
use anyhow::{Context, Result};
use headers::Header;
use headers_accept::Accept;
use hyper::body::Incoming;
use hyper::{Request, StatusCode, header};
use mediatype::{MediaType, ReadParams, WriteParams};
use prometheus_client::encoding::prometheus_protobuf;
use prometheus_client::encoding::text::encode as encode_openmetrics;
use prometheus_client::registry::Registry;

use super::hyper_helpers;
use crate::Address;
use crate::http::Response;

pub struct Server {
	s: hyper_helpers::Server<Registry>,
}

impl Server {
	pub async fn new(addr: Address, drain_rx: DrainWatcher, registry: Registry) -> Result<Self> {
		hyper_helpers::Server::<Registry>::bind("stats", addr, drain_rx, registry)
			.await
			.map(|s| Server { s })
	}

	pub fn address(&self) -> Option<SocketAddr> {
		self.s.address()
	}

	pub fn spawn(self) {
		self.s.spawn(|registry, req| async move {
			match req.uri().path() {
				"/metrics" | "/stats/prometheus" => handle_metrics(registry, req).await,
				_ => Ok(hyper_helpers::empty_response(hyper::StatusCode::NOT_FOUND)),
			}
		})
	}
}

async fn handle_metrics(reg: Arc<Registry>, req: Request<Incoming>) -> Result<Response> {
	let format = negotiate_format(&req).unwrap_or_default();

	let body = tokio::task::spawn_blocking(move || format.encode(&reg))
		.await
		.context("metrics encoding failed")??;

	Ok(
		::http::Response::builder()
			.status(StatusCode::OK)
			.header(header::CONTENT_TYPE, format.content_type())
			.header(header::VARY, header::ACCEPT.as_str())
			.body(body.into())?,
	)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum MetricsFormat {
	#[default]
	PlainText,
	OpenMetricsText,
	PrometheusProtobuf,
}

impl MetricsFormat {
	fn content_type(self) -> &'static str {
		match self {
			Self::PlainText => "text/plain;charset=utf-8",
			Self::OpenMetricsText => {
				"application/openmetrics-text;version=1.0.0;charset=utf-8;escaping=allow-utf-8"
			},
			Self::PrometheusProtobuf => {
				"application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited"
			},
		}
	}

	fn encode(self, registry: &Registry) -> Result<Vec<u8>> {
		match self {
			Self::OpenMetricsText | Self::PlainText => {
				let mut buffer = String::new();
				encode_openmetrics(&mut buffer, registry)?;
				Ok(buffer.into_bytes())
			},
			Self::PrometheusProtobuf => Ok(prometheus_protobuf::encode_to_vec(registry)?),
		}
	}
}

fn negotiate_format<T>(req: &Request<T>) -> Option<MetricsFormat> {
	use mediatype::Name;
	use mediatype::names::{APPLICATION, PLAIN, Q, TEXT};

	let mut values = req.headers().get_all(http::header::ACCEPT).iter();
	Accept::decode(&mut values)
		.ok()?
		.media_types()
		.map(|media_type| {
			let mut normalized = media_type.essence();
			if let Some(quality) = media_type.get_param(Q) {
				normalized.set_param(Q, quality);
			}
			normalized
		})
		.collect::<Accept>()
		.negotiate(&[
			MediaType::new(TEXT, PLAIN),
			MediaType::new(APPLICATION, Name::new_unchecked("openmetrics-text")),
			MediaType::new(APPLICATION, Name::new_unchecked("vnd.google.protobuf")),
			MediaType::new(APPLICATION, Name::new_unchecked("protobuf")),
			MediaType::new(APPLICATION, Name::new_unchecked("x-protobuf")),
		])
		.map(|media_type| match media_type.subty.as_str() {
			"openmetrics-text" => MetricsFormat::OpenMetricsText,
			"vnd.google.protobuf" | "protobuf" | "x-protobuf" => MetricsFormat::PrometheusProtobuf,
			_ => MetricsFormat::PlainText,
		})
}

#[cfg(test)]
mod tests {
	use std::net::{Ipv4Addr, SocketAddr};

	use prometheus_client::encoding::prometheus_protobuf::prometheus_data_model::MetricFamily;
	use prometheus_client::metrics::counter::Counter;
	use prometheus_client::registry::Registry;
	use prost::Message;
	use rstest::rstest;

	use super::{MetricsFormat, Server};
	use crate::Address;

	#[rstest]
	#[case::no_accept(None, MetricsFormat::PlainText)]
	#[case::wildcard(Some("*/*"), MetricsFormat::PlainText)]
	#[case::openmetrics(
		Some("application/openmetrics-text;version=1.0.0"),
		MetricsFormat::OpenMetricsText
	)]
	#[case::plain_text(Some("text/plain;version=0.0.4"), MetricsFormat::PlainText)]
	#[case::canonical_protobuf(
		Some(
			"APPLICATION/VND.GOOGLE.PROTOBUF;PROTO=io.prometheus.client.MetricFamily;ENCODING=delimited"
		),
		MetricsFormat::PrometheusProtobuf
	)]
	#[case::protobuf_alias(Some("application/protobuf"), MetricsFormat::PrometheusProtobuf)]
	#[case::x_protobuf_alias(Some("application/x-protobuf"), MetricsFormat::PrometheusProtobuf)]
	fn negotiates_supported_formats(#[case] accept: Option<&str>, #[case] expected: MetricsFormat) {
		let mut request = http::Request::new(());
		if let Some(accept) = accept {
			request
				.headers_mut()
				.insert(http::header::ACCEPT, accept.parse().unwrap());
		}
		assert_eq!(
			super::negotiate_format(&request).unwrap_or_default(),
			expected
		);
	}

	#[rstest]
	#[case::openmetrics_preferred(
		"application/openmetrics-text;version=1.0.0;escaping=allow-utf-8;q=0.5,application/openmetrics-text;version=0.0.1;q=0.4,text/plain;version=1.0.0;escaping=allow-utf-8;q=0.3,text/plain;version=0.0.4;q=0.2,*/*;q=0.1",
		MetricsFormat::OpenMetricsText
	)]
	#[case::protobuf_preferred(
		"application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited;q=0.6,application/openmetrics-text;version=1.0.0;escaping=allow-utf-8;q=0.5,application/openmetrics-text;version=0.0.1;q=0.4,text/plain;version=1.0.0;escaping=allow-utf-8;q=0.3,text/plain;version=0.0.4;q=0.2,*/*;q=0.1",
		MetricsFormat::PrometheusProtobuf
	)]
	fn negotiates_prometheus_scrape_headers(#[case] accept: &str, #[case] expected: MetricsFormat) {
		let request = http::Request::builder()
			.header(http::header::ACCEPT, accept)
			.body(())
			.unwrap();
		assert_eq!(
			super::negotiate_format(&request).unwrap_or_default(),
			expected,
			"{accept}"
		);
	}

	#[rstest]
	#[case::quality(
		"application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited;q=0.6,application/openmetrics-text;version=1.0.0;q=0.5,text/plain;version=0.0.4;q=0.4",
		MetricsFormat::PrometheusProtobuf
	)]
	#[case::bare_openmetrics(
		"application/openmetrics-text;q=0.9,text/plain;q=0.8",
		MetricsFormat::OpenMetricsText
	)]
	#[case::bare_protobuf(
		"application/vnd.google.protobuf;q=0.9,text/plain;q=0.8",
		MetricsFormat::PrometheusProtobuf
	)]
	#[case::wildcard_parameter("*/*;unknown=parameter", MetricsFormat::PlainText)]
	#[case::quoted_openmetrics_parameters(
		"application/openmetrics-text;version=\"1.0.0\";escaping=\"allow-utf-8\"",
		MetricsFormat::OpenMetricsText
	)]
	#[case::quoted_protobuf_parameters(
		"application/vnd.google.protobuf;proto=\"io.prometheus.client.MetricFamily\";encoding=\"delimited\"",
		MetricsFormat::PrometheusProtobuf
	)]
	fn honors_quality_and_representation_parameters(
		#[case] accept: &str,
		#[case] expected: MetricsFormat,
	) {
		let request = http::Request::builder()
			.header(http::header::ACCEPT, accept)
			.body(())
			.unwrap();
		assert_eq!(
			super::negotiate_format(&request).unwrap_or_default(),
			expected,
			"{accept}"
		);
	}

	#[tokio::test]
	async fn serves_each_metrics_format() {
		let counter: Counter = Counter::default();
		let mut registry = Registry::default();
		registry.register("requests", "Requests", counter.clone());
		counter.inc();

		let (_drain_tx, drain_rx) = agent_core::drain::new();
		let server = Server::new(
			Address::SocketAddr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
			drain_rx,
			registry,
		)
		.await
		.unwrap();
		let address = server.address().unwrap();
		server.spawn();

		let client = reqwest::Client::new();
		let url = format!("http://{address}/metrics");
		let openmetrics = client
			.get(&url)
			.header(
				"accept",
				"application/openmetrics-text;version=1.0.0;escaping=allow-utf-8",
			)
			.send()
			.await
			.unwrap();
		assert_eq!(openmetrics.status(), reqwest::StatusCode::OK);
		assert_eq!(
			openmetrics.headers()[reqwest::header::CONTENT_TYPE],
			"application/openmetrics-text;version=1.0.0;charset=utf-8;escaping=allow-utf-8"
		);
		assert!(openmetrics.bytes().await.unwrap().ends_with(b"# EOF\n"));

		let plain_text = client
			.get(&url)
			.header("accept", "text/plain;version=0.0.4")
			.send()
			.await
			.unwrap();
		assert_eq!(plain_text.status(), reqwest::StatusCode::OK);
		assert_eq!(
			plain_text.headers()[reqwest::header::CONTENT_TYPE],
			"text/plain;charset=utf-8"
		);
		assert!(plain_text.text().await.unwrap().contains("requests_total"));

		let protobuf = client
			.get(url)
			.header(
				"accept",
				"application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited",
			)
			.send()
			.await
			.unwrap();
		assert_eq!(protobuf.status(), reqwest::StatusCode::OK);
		assert_eq!(
			protobuf.headers()[reqwest::header::CONTENT_TYPE],
			"application/vnd.google.protobuf;proto=io.prometheus.client.MetricFamily;encoding=delimited"
		);
		let protobuf = protobuf.bytes().await.unwrap();
		let family = MetricFamily::decode_length_delimited(protobuf.as_ref()).unwrap();
		assert_eq!(family.name, "requests_total");
	}
}
