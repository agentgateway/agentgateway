use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone, Debug, Ord, PartialOrd)]
pub enum StringValue<'a> {
    Borrowed(&'a str),
    Owned(Arc<str>),
}

impl<'a> StringValue<'a> {
    pub fn as_owned(&self) -> Arc<str> {
        match self {
            StringValue::Borrowed(v) => Arc::from(*v),
            StringValue::Owned(o) => Arc::clone(o),
        }
    }
}

impl<'a> Deref for StringValue<'a> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_ref()
    }
}
impl AsRef<str> for StringValue<'_> {
    fn as_ref(&self) -> &str {
        match self {
            StringValue::Borrowed(s) => s,
            StringValue::Owned(s) => s.as_ref(),
        }
    }
}

impl Eq for StringValue<'_> {}
impl PartialEq for StringValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}
impl Hash for StringValue<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash only the string content, ignoring Borrowed vs Owned
        self.as_ref().hash(state);
    }
}

impl From<String> for StringValue<'static> {
    fn from(v: String) -> Self {
        StringValue::Owned(v.into())
    }
}

impl<'a> From<&'a str> for StringValue<'a> {
    fn from(v: &'a str) -> Self {
        StringValue::Borrowed(v)
    }
}

impl From<Arc<str>> for StringValue<'static> {
    fn from(v: Arc<str>) -> Self {
        StringValue::Owned(v)
    }
}
