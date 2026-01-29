use cel_derive::DynamicType;
pub struct CustomType(String);
fn extract_inner<'a>(c: &'a &'a CustomType) -> &'a String {
    &c.0
}
fn to_value<'a, T: AsRef<str>>(c: &'a &'a T) -> ::cel::Value<'a> {
    ::cel::Value::String(c.as_ref().into())
}
pub struct WithHelpers<'a> {
    #[dynamic(with = "extract_inner")]
    custom: &'a CustomType,
    #[dynamic(with_value = "to_value")]
    method: &'a str,
}
impl<'a> ::cel::types::dynamic::DynamicType for WithHelpers<'a> {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(2usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "custom" => {
                ::core::option::Option::Some({
                    let __field_ref: &&'a CustomType = &self.custom;
                    ::cel::types::dynamic::maybe_materialize(
                        (extract_inner)(__field_ref),
                    )
                })
            }
            "method" => ::core::option::Option::Some((to_value)(&self.method)),
            _ => ::core::option::Option::None,
        }
    }
}
impl<'a> ::cel::types::dynamic::DynamicFlatten for WithHelpers<'a> {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("custom"),
                ::cel::types::dynamic::maybe_materialize((extract_inner)(&self.custom)),
            );
        __cel_map
            .insert(::cel::objects::KeyRef::from("method"), (to_value)(&self.method));
    }
}
