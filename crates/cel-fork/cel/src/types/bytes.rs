use std::sync::Arc;

use bytes::Bytes;

#[derive(Clone, Debug)]
pub enum BytesValue<'a> {
    Borrowed(&'a [u8]),
    Owned(Arc<[u8]>),
    Bytes(Bytes),
}

impl From<Arc<[u8]>> for BytesValue<'static> {
    fn from(v: Arc<[u8]>) -> Self {
        BytesValue::Owned(v)
    }
}
impl<'a> AsRef<[u8]> for BytesValue<'a> {
    fn as_ref(&self) -> &[u8] {
        match self {
            BytesValue::Borrowed(b) => b,
            BytesValue::Owned(v) => v.as_ref(),
            BytesValue::Bytes(b) => b.as_ref(),
        }
    }
}

impl Eq for BytesValue<'_> {}
impl PartialEq for BytesValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}
