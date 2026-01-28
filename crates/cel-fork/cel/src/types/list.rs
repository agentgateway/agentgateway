use std::slice;
use std::sync::Arc;

use crate::Value;

#[derive(Debug)]
pub enum ListValue<'a> {
    Borrowed(&'a [Value<'a>]),
    PartiallyOwned(Arc<[Value<'a>]>),
    Owned(Arc<[Value<'static>]>),
}

impl<'a> ListValue<'a> {
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn len(&self) -> usize {
        match self {
            ListValue::Borrowed(items) => items.len(),
            ListValue::PartiallyOwned(items) => items.len(),
            ListValue::Owned(items) => items.len(),
        }
    }

    pub fn iter(&'a self) -> slice::Iter<'a, Value<'a>> {
        self.as_ref().iter()
    }
}

impl From<Arc<[Value<'static>]>> for ListValue<'static> {
    fn from(v: Arc<[Value<'static>]>) -> Self {
        ListValue::Owned(v)
    }
}

impl<'a> Clone for ListValue<'a> {
    fn clone(&self) -> Self {
        match self {
            ListValue::Borrowed(items) => ListValue::Borrowed(items),
            ListValue::PartiallyOwned(items) => ListValue::PartiallyOwned(items.clone()),
            ListValue::Owned(items) => ListValue::Owned(items.clone()),
        }
    }
}

impl<'a> AsRef<[Value<'a>]> for ListValue<'a> {
    fn as_ref(&self) -> &[Value<'a>] {
        match self {
            ListValue::Borrowed(a) => a,
            ListValue::PartiallyOwned(a) => a.as_ref(),
            ListValue::Owned(a) => a.as_ref(),
        }
    }
}

impl Eq for ListValue<'_> {}
impl PartialEq for ListValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}
