use crate::mcp::upstream::UpstreamError;
use rmcp::model::{ClientRequest, JsonRpcMessage, JsonRpcRequest};
use rmcp::transport::{TokioChildProcess, Transport};
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, Mutex};

pub struct Process {
	inner: Arc<Mutex<TokioChildProcess>>,
}

impl Process {
	pub async fn send_message(
		&self,
		req: JsonRpcRequest<ClientRequest>,
	) -> Result<(), UpstreamError> {
		let mut stream = self.inner.lock().unwrap();
		stream.send(JsonRpcMessage::Request(req)).await?;
		todo!()
	}
}

impl Process {
	pub fn new(mut proc: TokioChildProcess) -> Self {
		let recv = proc.receive();
		Self {
			inner: Arc::new(Mutex::new(proc)),
		}
	}
}

impl Debug for Process {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Process").finish()
	}
}
