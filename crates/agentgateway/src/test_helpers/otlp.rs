use std::net::SocketAddr;
use std::sync::Mutex;

use opentelemetry_proto::tonic::collector::trace::v1::trace_service_server::{
	TraceService, TraceServiceServer,
};
use opentelemetry_proto::tonic::collector::trace::v1::{
	ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::tonic::common::v1::any_value::Value as AnyValue;
use tokio::sync::mpsc;
use tonic::{Request, Response, Status};

#[derive(Debug)]
struct MockTraceService {
	tx: Mutex<mpsc::Sender<ExportTraceServiceRequest>>,
}

#[tonic::async_trait]
impl TraceService for MockTraceService {
	async fn export(
		&self,
		request: Request<ExportTraceServiceRequest>,
	) -> Result<Response<ExportTraceServiceResponse>, Status> {
		self
			.tx
			.lock()
			.expect("trace export sender mutex poisoned")
			.try_send(request.into_inner())
			.map_err(|e| Status::internal(format!("failed to capture export request: {e}")))?;
		Ok(Response::new(ExportTraceServiceResponse {
			partial_success: None,
		}))
	}
}

pub async fn start_mock_trace_collector() -> (SocketAddr, mpsc::Receiver<ExportTraceServiceRequest>)
{
	let listener = tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
		.await
		.expect("bind mock trace collector");
	let addr = listener.local_addr().expect("mock collector local addr");

	let (tx, rx) = mpsc::channel(16);
	let service = MockTraceService { tx: Mutex::new(tx) };
	tokio::spawn(async move {
		tonic::transport::Server::builder()
			.add_service(TraceServiceServer::new(service))
			.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
			.await
			.expect("mock trace collector server failed");
	});

	(addr, rx)
}

pub fn export_requests_spans(
	req: &ExportTraceServiceRequest,
) -> impl Iterator<Item = &opentelemetry_proto::tonic::trace::v1::Span> {
	req
		.resource_spans
		.iter()
		.flat_map(|rs| rs.scope_spans.iter())
		.flat_map(|ss| ss.spans.iter())
}

pub fn span_attr<'a>(
	span: &'a opentelemetry_proto::tonic::trace::v1::Span,
	key: &str,
) -> Option<&'a AnyValue> {
	span.attributes.iter().find_map(|kv| {
		if kv.key == key {
			kv.value.as_ref().and_then(|v| v.value.as_ref())
		} else {
			None
		}
	})
}

pub fn event_attr<'a>(
	event: &'a opentelemetry_proto::tonic::trace::v1::span::Event,
	key: &str,
) -> Option<&'a AnyValue> {
	event.attributes.iter().find_map(|kv| {
		if kv.key == key {
			kv.value.as_ref().and_then(|v| v.value.as_ref())
		} else {
			None
		}
	})
}

pub fn any_string(v: Option<&AnyValue>) -> Option<&str> {
	match v {
		Some(AnyValue::StringValue(s)) => Some(s.as_str()),
		_ => None,
	}
}

pub fn any_bool(v: Option<&AnyValue>) -> Option<bool> {
	match v {
		Some(AnyValue::BoolValue(b)) => Some(*b),
		Some(AnyValue::StringValue(s)) => s.parse::<bool>().ok(),
		_ => None,
	}
}

pub fn any_i64(v: Option<&AnyValue>) -> Option<i64> {
	match v {
		Some(AnyValue::IntValue(i)) => Some(*i),
		Some(AnyValue::StringValue(s)) => s.parse::<i64>().ok(),
		_ => None,
	}
}
