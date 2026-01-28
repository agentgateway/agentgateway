use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct WithFlatten {
    key: String,
    #[dynamic(flatten)]
    metadata: serde_json::Value,
}
