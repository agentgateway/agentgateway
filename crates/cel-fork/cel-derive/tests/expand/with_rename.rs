use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct WithRename {
    #[dynamic(rename = "firstName")]
    first_name: String,
    #[dynamic(rename = "lastName")]
    last_name: String,
}
