use cel_derive::DynamicType;
pub struct WithFlatten {
    key: String,
    #[dynamic(flatten)]
    metadata: serde_json::Value,
}
impl ::cel::types::dynamic::DynamicType for WithFlatten {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(1usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "key" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.key),
                )
            }
            _ => {
                if let ::core::option::Option::Some(val) = ::cel::types::dynamic::DynamicType::field(
                    &self.metadata,
                    field,
                ) {
                    return ::core::option::Option::Some(val);
                }
                ::core::option::Option::None
            }
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithFlatten {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("key"),
                ::cel::types::dynamic::maybe_materialize(&self.key),
            );
        ::cel::types::dynamic::DynamicFlatten::materialize_into(
            &self.metadata,
            __cel_map,
        );
    }
}
