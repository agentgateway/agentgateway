use cel_derive::DynamicType;
pub struct WithSkip {
    public_field: String,
    #[dynamic(skip)]
    internal_field: u64,
}
impl ::cel::types::dynamic::DynamicType for WithSkip {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(1usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "public_field" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.public_field),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithSkip {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("public_field"),
                ::cel::types::dynamic::maybe_materialize(&self.public_field),
            );
    }
}
