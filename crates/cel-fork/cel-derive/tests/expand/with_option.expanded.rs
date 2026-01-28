use cel_derive::DynamicType;
pub struct WithOption {
    required: String,
    optional: Option<String>,
    nested_optional: Option<i32>,
}
impl ::cel::types::dynamic::DynamicType for WithOption {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(3usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "required" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.required),
                )
            }
            "optional" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.optional),
                )
            }
            "nested_optional" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.nested_optional),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithOption {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("required"),
                ::cel::types::dynamic::maybe_materialize(&self.required),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("optional"),
                ::cel::types::dynamic::maybe_materialize(&self.optional),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("nested_optional"),
                ::cel::types::dynamic::maybe_materialize(&self.nested_optional),
            );
    }
}
