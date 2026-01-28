use std::any::Any;

use crate::common::types::Type;
use crate::common::value::Val;

#[derive(Clone, Debug, PartialEq)]
pub struct Bool(bool);

impl Val for Bool {
    fn get_type(&self) -> Type<'_> {
        super::BOOL_TYPE
    }

    fn into_inner(self) -> Box<dyn Any> {
        Box::new(self.0)
    }
}

impl From<Bool> for bool {
    fn from(value: Bool) -> Self {
        value.0
    }
}

impl From<bool> for Bool {
    fn from(value: bool) -> Self {
        Bool(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::types;
    use crate::common::types::Kind;

    #[test]
    fn test_from() {
        let value: Bool = true.into();
        let v: bool = value.into();
        assert!(v)
    }

    #[test]
    fn test_type() {
        let value = Bool(true);
        assert_eq!(value.get_type(), types::BOOL_TYPE);
        assert_eq!(value.get_type().kind, Kind::Boolean);
    }

    #[test]
    fn test_into_inner() {
        let value = Bool(true);
        let inner = value.into_inner();
        let option = inner.downcast::<bool>();
        let b = option.unwrap();
        assert!(*b);
    }
}
