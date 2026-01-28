use cel_derive::DynamicType;
pub struct ExtAuthzDynamicMetadata(serde_json::Map<String, serde_json::Value>);
impl ::cel::types::dynamic::DynamicType for ExtAuthzDynamicMetadata {
    fn auto_materialize(&self) -> bool {
        self.0.auto_materialize()
    }
    fn materialize(&self) -> ::cel::Value<'_> {
        self.0.materialize()
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        self.0.field(field)
    }
}
impl ::cel::types::dynamic::DynamicFlatten for ExtAuthzDynamicMetadata {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        ::cel::types::dynamic::DynamicFlatten::materialize_into(&self.0, __cel_map);
    }
}
