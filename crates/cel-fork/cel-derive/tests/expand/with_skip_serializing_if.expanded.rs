use cel_derive::DynamicType;
pub struct WithSkipSerializingIf<'a> {
    required: &'a str,
    #[dynamic(skip_serializing_if = "Option::is_none")]
    optional: Option<&'a str>,
    #[serde(skip_serializing_if = "str::is_empty")]
    name: &'a str,
}
impl<'a> ::cel::types::dynamic::DynamicType for WithSkipSerializingIf<'a> {
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
                if (Option::is_none)(&self.optional) {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(
                        ::cel::types::dynamic::maybe_materialize(&self.optional),
                    )
                }
            }
            "name" => {
                if (str::is_empty)(&self.name) {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(
                        ::cel::types::dynamic::maybe_materialize(&self.name),
                    )
                }
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl<'a> ::cel::types::dynamic::DynamicFlatten for WithSkipSerializingIf<'a> {
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
        if !(Option::is_none)(&self.optional) {
            __cel_map
                .insert(
                    ::cel::objects::KeyRef::from("optional"),
                    ::cel::types::dynamic::maybe_materialize(&self.optional),
                );
        }
        if !(str::is_empty)(&self.name) {
            __cel_map
                .insert(
                    ::cel::objects::KeyRef::from("name"),
                    ::cel::types::dynamic::maybe_materialize(&self.name),
                );
        }
    }
}
