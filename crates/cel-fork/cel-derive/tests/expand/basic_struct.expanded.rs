use cel_derive::DynamicType;
pub struct BasicStruct {
    name: String,
    age: i32,
    active: bool,
}
impl ::cel::types::dynamic::DynamicType for BasicStruct {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(3usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "name" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.name),
                )
            }
            "age" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.age),
                )
            }
            "active" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.active),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for BasicStruct {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("name"),
                ::cel::types::dynamic::maybe_materialize(&self.name),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("age"),
                ::cel::types::dynamic::maybe_materialize(&self.age),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("active"),
                ::cel::types::dynamic::maybe_materialize(&self.active),
            );
    }
}
