use cel_derive::DynamicType;

fn is_none<T>(opt: &Option<T>) -> bool {
    opt.is_none()
}

#[derive(DynamicType)]
pub struct WithSerdeMultiArgs {
    #[serde(rename = "apiKey", skip_serializing_if = "is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "is_none", rename = "userId")]
    user_id: Option<i32>,
    normal_field: String,
}
