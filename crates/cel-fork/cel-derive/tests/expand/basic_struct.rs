use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct BasicStruct {
    name: String,
    age: i32,
    active: bool,
}
