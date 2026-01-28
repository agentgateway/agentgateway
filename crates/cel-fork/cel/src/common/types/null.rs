use std::any::Any;

use crate::common::types::Type;
use crate::common::value::Val;

pub struct Null;

impl Val for Null {
    fn get_type(&self) -> Type<'_> {
        super::NULL_TYPE
    }

    fn into_inner(self) -> Box<dyn Any> {
        Box::new(None::<()>)
    }
}
