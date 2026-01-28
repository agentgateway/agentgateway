use cel_derive::DynamicType;

#[derive(DynamicType)]
#[dynamic(rename_all = "camelCase")]
pub struct WithRenameAllCamel {
    user_name: String,
    user_age: i32,
    is_active: bool,
}
