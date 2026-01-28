use cel_derive::DynamicType;
#[serde(rename_all = "camelCase")]
pub struct WithSerdeRenameAll {
    user_name: String,
    user_id: i32,
}
impl ::cel::types::dynamic::DynamicType for WithSerdeRenameAll {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(2usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "userName" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.user_name),
                )
            }
            "userId" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.user_id),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithSerdeRenameAll {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("userName"),
                ::cel::types::dynamic::maybe_materialize(&self.user_name),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("userId"),
                ::cel::types::dynamic::maybe_materialize(&self.user_id),
            );
    }
}
