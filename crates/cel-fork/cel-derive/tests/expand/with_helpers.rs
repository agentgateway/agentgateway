use cel_derive::DynamicType;

// Mock types for testing
pub struct CustomType(String);

fn extract_inner<'a>(c: &'a &'a CustomType) -> &'a String {
    &c.0
}

fn to_value<'a, T: AsRef<str>>(c: &'a &'a T) -> ::cel::Value<'a> {
    ::cel::Value::String(c.as_ref().into())
}

#[derive(DynamicType)]
pub struct WithHelpers<'a> {
    #[dynamic(with = "extract_inner")]
    custom: &'a CustomType,
    #[dynamic(with_value = "to_value")]
    method: &'a str,
}
