use rmcp::transport::TokioChildProcess;
use std::fmt;
use std::fmt::{Debug, Formatter};

pub struct Process {
	inner: TokioChildProcess,
}

impl Process {
	pub fn new(inner: TokioChildProcess) -> Self {
		Self { inner }
	}
}

impl Debug for Process {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Process").finish()
	}
}
