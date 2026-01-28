use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct WithOption {
    required: String,
    optional: Option<String>,
    nested_optional: Option<i32>,
}
