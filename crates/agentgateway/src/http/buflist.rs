use std::collections::VecDeque;
use std::io::IoSlice;

use bytes::{Buf, BufMut, Bytes, BytesMut};

// Liberated from http_body
// BufList is a list of buffers. It is clone-able, but *not* cheaply
#[derive(Clone, Debug)]
pub struct BufList<T = Bytes> {
	bufs: VecDeque<T>,
}

impl<T: Buf> BufList<T> {
	#[inline]
	pub(crate) fn push(&mut self, buf: T) {
		debug_assert!(buf.has_remaining());
		self.bufs.push_back(buf);
	}

	#[inline]
	pub fn get_chunk(&mut self, idx: usize) -> Option<&T> {
		self.bufs.get(idx)
	}

	#[inline]
	pub fn pop_front(&mut self) -> Option<T> {
		self.bufs.pop_front()
	}
}

impl<T: Buf> Buf for BufList<T> {
	#[inline]
	fn remaining(&self) -> usize {
		self.bufs.iter().map(|buf| buf.remaining()).sum()
	}

	#[inline]
	fn chunk(&self) -> &[u8] {
		self.bufs.front().map(Buf::chunk).unwrap_or_default()
	}

	#[inline]
	fn chunks_vectored<'t>(&'t self, dst: &mut [IoSlice<'t>]) -> usize {
		if dst.is_empty() {
			return 0;
		}
		let mut vecs = 0;
		for buf in &self.bufs {
			vecs += buf.chunks_vectored(&mut dst[vecs..]);
			if vecs == dst.len() {
				break;
			}
		}
		vecs
	}

	#[inline]
	fn advance(&mut self, mut cnt: usize) {
		while cnt > 0 {
			{
				let front = &mut self.bufs[0];
				let rem = front.remaining();
				if rem > cnt {
					front.advance(cnt);
					return;
				} else {
					front.advance(rem);
					cnt -= rem;
				}
			}
			self.bufs.pop_front();
		}
	}

	#[inline]
	fn has_remaining(&self) -> bool {
		self.bufs.iter().any(|buf| buf.has_remaining())
	}

	#[inline]
	fn copy_to_bytes(&mut self, len: usize) -> Bytes {
		// Our inner buffer may have an optimized version of copy_to_bytes, and if the whole
		// request can be fulfilled by the front buffer, we can take advantage.
		match self.bufs.front_mut() {
			Some(front) if front.remaining() == len => {
				let b = front.copy_to_bytes(len);
				self.bufs.pop_front();
				b
			},
			Some(front) if front.remaining() > len => front.copy_to_bytes(len),
			_ => {
				let rem = self.remaining();
				assert!(len <= rem, "`len` greater than remaining");
				let mut bm = BytesMut::with_capacity(len);
				if rem == len {
					// .take() costs a lot more, so skip it if we don't need it
					bm.put(self);
				} else {
					bm.put(self.take(len));
				}
				bm.freeze()
			},
		}
	}
}

impl<T> Default for BufList<T> {
	fn default() -> Self {
		BufList {
			bufs: VecDeque::new(),
		}
	}
}
