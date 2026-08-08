//! Envoy-compatible L4 external processing for TCP streams.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::http::ext_proc::GrpcReferenceChannel;
use crate::http::metadata_context::MetadataContext;
use crate::proxy::httpproxy::PolicyClient;
use crate::types::agent::SimpleBackendReferenceWithPolicies;
use crate::{cel, *};

const NETWORK_EXTPROC_ATTRIBUTES_NAMESPACE: &str = "envoy.filters.network.ext_proc";

pub mod proto {
	pub use protos::envoy::service::network_ext_proc::v3::*;
}

type Metadata = protos::envoy::config::core::v3::Metadata;

#[apply(schema!)]
#[derive(Default, Copy, PartialEq, Eq)]
pub enum DataSendMode {
	#[default]
	Streamed,
	Skip,
}

#[apply(schema!)]
pub struct NetworkExtProc {
	/// Service implementing Envoy's NetworkExternalProcessor gRPC protocol.
	pub target: SimpleBackendReferenceWithPolicies,
	/// Continue proxying unprocessed bytes if the processor cannot be reached.
	#[serde(default, skip_serializing_if = "crate::serdes::is_default")]
	pub failure_mode_allow: bool,
	#[serde(default, skip_serializing_if = "crate::serdes::is_default")]
	pub process_read: DataSendMode,
	#[serde(default, skip_serializing_if = "crate::serdes::is_default")]
	pub process_write: DataSendMode,
	/// Maximum time to wait for each processor response. Defaults to Envoy's 200ms.
	#[serde(default)]
	pub message_timeout_ms: u64,
	/// CEL-generated metadata sent with every network processing request, grouped
	/// by the user-defined Envoy metadata namespace.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub metadata_context: Option<MetadataContext>,
	/// CEL-generated connection attributes sent in ProcessingRequest.attributes.
	///
	/// This is wire-compatible with Envoy's network ext_proc connection_attributes
	/// output. We intentionally evaluate user-provided CEL expressions directly
	/// instead of trying to model Envoy filter state.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub connection_attributes: Option<HashMap<String, Arc<cel::Expression>>>,
}

impl NetworkExtProc {
	pub fn evaluate_request_context(&self, exec: &cel::Executor<'_>) -> RequestContext {
		let filter_metadata = crate::http::metadata_context::build_processing_metadata_context(
			exec,
			self.metadata_context.as_ref(),
		)
		.unwrap_or_default();
		let attributes = self
			.connection_attributes
			.as_ref()
			.and_then(|attrs| crate::http::metadata_context::eval_to_struct(exec, attrs).ok())
			.filter(|attrs| !attrs.fields.is_empty())
			.map(|attrs| HashMap::from([(NETWORK_EXTPROC_ATTRIBUTES_NAMESPACE.to_string(), attrs)]))
			.unwrap_or_default();
		RequestContext {
			metadata: Metadata {
				filter_metadata,
				typed_filter_metadata: HashMap::new(),
			},
			attributes,
		}
	}
}

#[derive(Default, Clone)]
pub struct RequestContext {
	pub metadata: Metadata,
	pub attributes: HashMap<String, prost_wkt_types::Struct>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
	#[error("network ext_proc stream failed: {0}")]
	Grpc(#[from] tonic::Status),
	#[error("network ext_proc stream closed before responding")]
	StreamClosed,
	#[error("network ext_proc response did not match the processed direction")]
	WrongDirection,
	#[error(transparent)]
	Io(#[from] std::io::Error),
}

#[derive(Clone)]
struct Processor {
	tx: mpsc::Sender<proto::ProcessingRequest>,
	rx: Arc<Mutex<mpsc::Receiver<Result<proto::ProcessingResponse, Error>>>>,
	serial: Arc<Mutex<()>>,
	timeout: Duration,
	context: RequestContext,
}

impl Processor {
	async fn connect(
		config: &NetworkExtProc,
		client: PolicyClient,
		context: RequestContext,
	) -> Result<Self, Error> {
		let channel: GrpcReferenceChannel = config.target.grpc_channel(client);
		let mut client =
			proto::network_external_processor_client::NetworkExternalProcessorClient::new(channel);
		let (tx, rx) = mpsc::channel(16);
		let (response_tx, response_rx) = mpsc::channel(16);
		tokio::spawn(async move {
			let result = client.process(ReceiverStream::new(rx)).await;
			match result {
				Ok(response) => {
					let mut response = response.into_inner();
					loop {
						match response.message().await {
							Ok(Some(message)) => {
								if response_tx.send(Ok(message)).await.is_err() {
									break;
								}
							},
							Ok(None) => break,
							Err(status) => {
								let _ = response_tx.send(Err(Error::Grpc(status))).await;
								break;
							},
						}
					}
				},
				Err(status) => {
					let _ = response_tx.send(Err(Error::Grpc(status))).await;
				},
			}
		});
		Ok(Self {
			tx,
			rx: Arc::new(Mutex::new(response_rx)),
			serial: Arc::new(Mutex::new(())),
			timeout: Duration::from_millis(config.message_timeout_ms.max(200)),
			context,
		})
	}

	async fn process(
		&self,
		read: bool,
		data: Vec<u8>,
		end_of_stream: bool,
	) -> Result<(Option<Vec<u8>>, bool), Error> {
		let _serial = self.serial.lock().await;
		let request = proto::ProcessingRequest {
			read_data: read.then(|| proto::Data {
				data: data.clone().into(),
				end_of_stream,
			}),
			write_data: (!read).then(|| proto::Data {
				data: data.into(),
				end_of_stream,
			}),
			metadata: Some(self.context.metadata.clone()),
			attributes: self.context.attributes.clone(),
		};
		self
			.tx
			.send(request)
			.await
			.map_err(|_| Error::StreamClosed)?;
		let response = tokio::time::timeout(self.timeout, self.rx.lock().await.recv())
			.await
			.map_err(|_| Error::StreamClosed)?
			.ok_or(Error::StreamClosed)??;
		let modified = response.data_processing_status
			== proto::processing_response::DataProcessedStatus::Modified as i32;
		let payload = if modified {
			if read {
				response.read_data
			} else {
				response.write_data
			}
			.ok_or(Error::WrongDirection)?
			.data
			.to_vec()
		} else {
			Vec::new()
		};
		let close =
			response.connection_status != proto::processing_response::ConnectionStatus::Continue as i32;
		Ok((modified.then_some(payload), close))
	}
}

/// Proxy a TCP connection through a NetworkExternalProcessor stream.
pub async fn proxy<A, B>(
	downstream: A,
	upstream: B,
	config: &NetworkExtProc,
	client: PolicyClient,
	context: RequestContext,
) -> Result<(), Error>
where
	A: AsyncRead + AsyncWrite + Unpin + Send + 'static,
	B: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
	let processor = Processor::connect(config, client, context).await?;
	let (dr, dw) = tokio::io::split(downstream);
	let (ur, uw) = tokio::io::split(upstream);
	let a = copy(dr, uw, processor.clone(), true, config.process_read);
	let b = copy(ur, dw, processor, false, config.process_write);
	let (a, b) = tokio::join!(a, b);
	a?;
	b?;
	Ok(())
}

async fn copy<R, W>(
	mut reader: R,
	mut writer: W,
	processor: Processor,
	read: bool,
	mode: DataSendMode,
) -> Result<(), Error>
where
	R: AsyncRead + Unpin,
	W: AsyncWrite + Unpin,
{
	let mut buf = vec![0; 16 * 1024];
	loop {
		let n = reader.read(&mut buf).await?;
		if n == 0 {
			if mode == DataSendMode::Streamed {
				let _ = processor.process(read, Vec::new(), true).await?;
			}
			writer.shutdown().await?;
			return Ok(());
		}
		let original = &buf[..n];
		let bytes = if mode == DataSendMode::Streamed {
			let (replacement, close) = processor.process(read, original.to_vec(), false).await?;
			if close {
				writer.shutdown().await?;
				return Ok(());
			}
			if let Some(replacement) = replacement {
				writer.write_all(&replacement).await?;
				continue;
			}
			original
		} else {
			original
		};
		writer.write_all(bytes).await?;
	}
}
