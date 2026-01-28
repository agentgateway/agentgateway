use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct ExtAuthzDynamicMetadata(serde_json::Map<String, serde_json::Value>);
