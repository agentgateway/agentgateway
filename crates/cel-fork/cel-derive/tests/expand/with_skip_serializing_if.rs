use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct WithSkipSerializingIf<'a> {
    required: &'a str,
    #[dynamic(skip_serializing_if = "Option::is_none")]
    optional: Option<&'a str>,
    #[serde(skip_serializing_if = "str::is_empty")]
    name: &'a str,
}
