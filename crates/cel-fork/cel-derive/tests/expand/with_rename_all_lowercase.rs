use cel_derive::DynamicType;

#[derive(DynamicType)]
#[dynamic(rename_all = "lowercase")]
pub struct WithRenameAllLower {
    UserName: String,
    UserAge: i32,
}
