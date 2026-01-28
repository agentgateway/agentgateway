use std::any::Any;

use crate::common::types::Type;
use crate::common::value::Val;

pub struct Int(i64);

impl Val for Int {
    fn get_type(&self) -> Type<'_> {
        super::INT_TYPE
    }

    fn into_inner(self) -> Box<dyn Any> {
        Box::new(self.0)
    }
}

impl From<Int> for i64 {
    fn from(value: Int) -> Self {
        value.0
    }
}

impl From<i64> for Int {
    fn from(value: i64) -> Self {
        Self(value)
    }
}
