use cel_derive::DynamicType;
#[dynamic(rename_all = "camelCase")]
pub struct WithRenameAllCamel {
    user_name: String,
    user_age: i32,
    is_active: bool,
}
impl ::cel::types::dynamic::DynamicType for WithRenameAllCamel {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(3usize);
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
            "userAge" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.user_age),
                )
            }
            "isActive" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.is_active),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithRenameAllCamel {
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
                ::cel::objects::KeyRef::from("userAge"),
                ::cel::types::dynamic::maybe_materialize(&self.user_age),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("isActive"),
                ::cel::types::dynamic::maybe_materialize(&self.is_active),
            );
    }
}
