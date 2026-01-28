use cel_derive::DynamicType;

#[derive(DynamicType)]
#[dynamic(rename_all = "camelCase")]
pub struct WithMixedAnnotations {
    user_name: String,
    #[serde(skip)]
    serde_skip_field: String,
    #[dynamic(skip)]
    dynamic_skip_field: String,
    #[serde(rename = "serdeCustom")]
    serde_rename: String,
    #[dynamic(rename = "dynamicCustom")]
    dynamic_rename: String,
    // Both present - dynamic should win
    #[serde(rename = "serdeWins")]
    #[dynamic(rename = "dynamicWins")]
    both_rename: String,
}
