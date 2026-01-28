use cel_derive::DynamicType;
pub struct WithRename {
    #[dynamic(rename = "firstName")]
    first_name: String,
    #[dynamic(rename = "lastName")]
    last_name: String,
}
impl ::cel::types::dynamic::DynamicType for WithRename {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(2usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "firstName" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.first_name),
                )
            }
            "lastName" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.last_name),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithRename {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("firstName"),
                ::cel::types::dynamic::maybe_materialize(&self.first_name),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("lastName"),
                ::cel::types::dynamic::maybe_materialize(&self.last_name),
            );
    }
}
