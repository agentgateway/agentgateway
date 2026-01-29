use cel_derive::DynamicType;

// ===== Basic struct types =====

#[derive(DynamicType)]
pub struct BasicStruct {
    name: String,
    age: i32,
    active: bool,
}

#[derive(DynamicType)]
pub struct NewtypeStruct(serde_json::Map<String, serde_json::Value>);

#[derive(DynamicType)]
pub struct WithOption {
    required: String,
    optional: Option<String>,
    nested_optional: Option<i32>,
}

// ===== Enums =====

#[derive(DynamicType)]
pub enum BasicEnum {
    Http,
    Grpc,
    WebSocket,
}

#[derive(DynamicType)]
pub enum EnumWithRename {
    #[dynamic(rename = "http")]
    Http,
    #[dynamic(rename = "grpc")]
    Grpc,
    #[dynamic(rename = "ws")]
    WebSocket,
}

// ===== Skip attributes =====

#[derive(DynamicType)]
pub struct WithSkip {
    public_field: String,
    #[dynamic(skip)]
    internal_field: u64,
}

#[derive(DynamicType)]
pub struct WithSkipSerializingIf<'a> {
    required: &'a str,
    #[dynamic(skip_serializing_if = "Option::is_none")]
    optional: Option<&'a str>,
    #[serde(skip_serializing_if = "str::is_empty")]
    name: &'a str,
}

// ===== Rename variants =====

#[derive(DynamicType)]
#[dynamic(rename_all = "camelCase")]
pub struct RenameAllCamel {
    user_name: String,
    user_age: i32,
}

#[derive(DynamicType)]
#[dynamic(rename_all = "lowercase")]
pub struct RenameAllLower {
    UserName: String,
    UserAge: i32,
}

#[derive(DynamicType)]
pub struct IndividualRename {
    #[dynamic(rename = "firstName")]
    first_name: String,
    #[dynamic(rename = "lastName")]
    last_name: String,
    age: i32,
}

// ===== Serde integration =====

fn is_none<T>(opt: &Option<T>) -> bool {
    opt.is_none()
}

#[derive(DynamicType)]
pub struct SerdeBasic {
    required: String,
    #[serde(skip)]
    internal_id: u64,
    #[serde(rename = "custom_name")]
    original_name: String,
}

#[derive(DynamicType)]
pub struct SerdeMultiArgs {
    #[serde(rename = "apiKey", skip_serializing_if = "is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "is_none", rename = "userId")]
    user_id: Option<i32>,
    normal_field: String,
}

#[derive(DynamicType)]
#[serde(rename_all = "camelCase")]
pub struct SerdeRenameAll {
    user_name: String,
    user_id: i32,
}

// ===== Flatten attribute =====

#[derive(DynamicType)]
pub struct WithFlatten {
    key: String,
    #[dynamic(flatten)]
    metadata: serde_json::Value,
}

// ===== Mixed annotations (precedence testing) =====

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
