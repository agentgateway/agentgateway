use cel_derive::DynamicType;
#[dynamic(rename_all = "lowercase")]
pub struct WithRenameAllLower {
    UserName: String,
    UserAge: i32,
}
impl ::cel::types::dynamic::DynamicType for WithRenameAllLower {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(2usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "username" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.UserName),
                )
            }
            "userage" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.UserAge),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithRenameAllLower {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("username"),
                ::cel::types::dynamic::maybe_materialize(&self.UserName),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("userage"),
                ::cel::types::dynamic::maybe_materialize(&self.UserAge),
            );
    }
}
