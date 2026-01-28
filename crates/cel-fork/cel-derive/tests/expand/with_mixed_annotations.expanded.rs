use cel_derive::DynamicType;
#[dynamic(rename_all = "camelCase")]
pub struct WithMixedAnnotations {
    user_name: String,
    #[serde(skip)]
    serde_skip_field: String,
    #[dynamic(skip)]
    dynamic_skip_field: String,
    #[serde(rename = "serdeCustom")]
    serde_rename: String,
    #[dynamic(rename = "dynamicCustom")]
    dynamic_rename: String,
    #[serde(rename = "serdeWins")]
    #[dynamic(rename = "dynamicWins")]
    both_rename: String,
}
impl ::cel::types::dynamic::DynamicType for WithMixedAnnotations {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(4usize);
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
            "serdeCustom" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.serde_rename),
                )
            }
            "dynamicCustom" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.dynamic_rename),
                )
            }
            "dynamicWins" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.both_rename),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithMixedAnnotations {
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
                ::cel::objects::KeyRef::from("serdeCustom"),
                ::cel::types::dynamic::maybe_materialize(&self.serde_rename),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("dynamicCustom"),
                ::cel::types::dynamic::maybe_materialize(&self.dynamic_rename),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("dynamicWins"),
                ::cel::types::dynamic::maybe_materialize(&self.both_rename),
            );
    }
}
