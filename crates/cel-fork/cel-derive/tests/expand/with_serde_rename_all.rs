use cel_derive::DynamicType;

#[derive(DynamicType)]
#[serde(rename_all = "camelCase")]
pub struct WithSerdeRenameAll {
    user_name: String,
    user_id: i32,
}
