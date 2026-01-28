use cel_derive::DynamicType;

#[derive(DynamicType)]
pub struct WithSkip {
    public_field: String,
    #[dynamic(skip)]
    internal_field: u64,
}
