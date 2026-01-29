// OwnedOrBorrowed is exactly like Cow but doesn't require the type to be Clone
#[derive(Debug, Clone)]
pub enum OwnedOrBorrowed<'a, T> {
	Borrowed(&'a T),
	Owned(T),
}

impl<'a, T> std::ops::Deref for OwnedOrBorrowed<'a, T> {
	type Target = T;

	fn deref(&self) -> &T {
		match self {
			Self::Borrowed(v) => v,
			Self::Owned(v) => v,
		}
	}
}

impl<'a, T> std::convert::AsRef<T> for OwnedOrBorrowed<'a, T> {
	fn as_ref(&self) -> &T {
		self
	}
}

impl<'a, T> std::convert::From<&'a T> for OwnedOrBorrowed<'a, T> {
	fn from(v: &'a T) -> Self {
		Self::Borrowed(v)
	}
}
impl<'a, T> std::convert::From<T> for OwnedOrBorrowed<'a, T> {
	fn from(v: T) -> Self {
		Self::Owned(v)
	}
}
