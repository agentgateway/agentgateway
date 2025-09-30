pub mod hbone;
pub mod stream;
pub mod tls;

#[derive(Debug, Clone)]
pub struct BufferLimit(pub usize);

impl BufferLimit {
	pub fn new(limit: usize) -> Self {
		BufferLimit(limit)
	}
}
