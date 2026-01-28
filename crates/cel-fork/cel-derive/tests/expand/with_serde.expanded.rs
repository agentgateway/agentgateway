use cel_derive::DynamicType;
pub struct WithSerde {
    required: String,
    #[serde(skip)]
    internal_id: u64,
    #[serde(rename = "custom_name")]
    original_name: String,
}
impl ::cel::types::dynamic::DynamicType for WithSerde {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(2usize);
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
            "custom_name" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.original_name),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithSerde {
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
                ::cel::objects::KeyRef::from("custom_name"),
                ::cel::types::dynamic::maybe_materialize(&self.original_name),
            );
    }
}
