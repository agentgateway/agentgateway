use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct WithSerde {
    required: String,
    #[serde(skip)]
    internal_id: u64,
    #[serde(rename = "custom_name")]
    original_name: String,
}
