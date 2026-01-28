use cel_derive::DynamicType;
fn is_none<T>(opt: &Option<T>) -> bool {
    opt.is_none()
}
pub struct WithSerdeMultiArgs {
    #[serde(rename = "apiKey", skip_serializing_if = "is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "is_none", rename = "userId")]
    user_id: Option<i32>,
    normal_field: String,
}
impl ::cel::types::dynamic::DynamicType for WithSerdeMultiArgs {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(3usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "apiKey" => {
                if (is_none)(&self.api_key) {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(
                        ::cel::types::dynamic::maybe_materialize(&self.api_key),
                    )
                }
            }
            "userId" => {
                if (is_none)(&self.user_id) {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(
                        ::cel::types::dynamic::maybe_materialize(&self.user_id),
                    )
                }
            }
            "normal_field" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.normal_field),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithSerdeMultiArgs {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        if !(is_none)(&self.api_key) {
            __cel_map
                .insert(
                    ::cel::objects::KeyRef::from("apiKey"),
                    ::cel::types::dynamic::maybe_materialize(&self.api_key),
                );
        }
        if !(is_none)(&self.user_id) {
            __cel_map
                .insert(
                    ::cel::objects::KeyRef::from("userId"),
                    ::cel::types::dynamic::maybe_materialize(&self.user_id),
                );
        }
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("normal_field"),
                ::cel::types::dynamic::maybe_materialize(&self.normal_field),
            );
    }
}
