use std::any::Any;

use crate::common::types::Type;
use crate::common::value::Val;

pub struct Double(f64);

impl Val for Double {
    fn get_type(&self) -> Type<'_> {
        super::DOUBLE_TYPE
    }

    fn into_inner(self) -> Box<dyn Any> {
        Box::new(self.0)
    }
}

impl From<Double> for f64 {
    fn from(value: Double) -> Self {
        value.0
    }
}

impl From<f64> for Double {
    fn from(value: f64) -> Self {
        Self(value)
    }
}
