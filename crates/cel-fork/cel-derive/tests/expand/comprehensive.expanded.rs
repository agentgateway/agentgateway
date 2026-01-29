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
pub struct NewtypeStruct(serde_json::Map<String, serde_json::Value>);
impl ::cel::types::dynamic::DynamicType for NewtypeStruct {
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
impl ::cel::types::dynamic::DynamicFlatten for NewtypeStruct {
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
pub struct WithOption {
    required: String,
    optional: Option<String>,
    nested_optional: Option<i32>,
}
impl ::cel::types::dynamic::DynamicType for WithOption {
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
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.optional),
                )
            }
            "nested_optional" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.nested_optional),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for WithOption {
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
                ::cel::objects::KeyRef::from("optional"),
                ::cel::types::dynamic::maybe_materialize(&self.optional),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("nested_optional"),
                ::cel::types::dynamic::maybe_materialize(&self.nested_optional),
            );
    }
}
pub enum BasicEnum {
    Http,
    Grpc,
    WebSocket,
}
impl ::cel::types::dynamic::DynamicType for BasicEnum {
    fn auto_materialize(&self) -> bool {
        true
    }
    fn materialize(&self) -> ::cel::Value<'_> {
        match self {
            Self::Http => ::cel::Value::String("Http".into()),
            Self::Grpc => ::cel::Value::String("Grpc".into()),
            Self::WebSocket => ::cel::Value::String("WebSocket".into()),
        }
    }
}
pub enum EnumWithRename {
    #[dynamic(rename = "http")]
    Http,
    #[dynamic(rename = "grpc")]
    Grpc,
    #[dynamic(rename = "ws")]
    WebSocket,
}
impl ::cel::types::dynamic::DynamicType for EnumWithRename {
    fn auto_materialize(&self) -> bool {
        true
    }
    fn materialize(&self) -> ::cel::Value<'_> {
        match self {
            Self::Http => ::cel::Value::String("http".into()),
            Self::Grpc => ::cel::Value::String("grpc".into()),
            Self::WebSocket => ::cel::Value::String("ws".into()),
        }
    }
}
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
#[dynamic(rename_all = "camelCase")]
pub struct RenameAllCamel {
    user_name: String,
    user_age: i32,
}
impl ::cel::types::dynamic::DynamicType for RenameAllCamel {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(2usize);
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
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for RenameAllCamel {
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
    }
}
#[dynamic(rename_all = "lowercase")]
pub struct RenameAllLower {
    UserName: String,
    UserAge: i32,
}
impl ::cel::types::dynamic::DynamicType for RenameAllLower {
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
impl ::cel::types::dynamic::DynamicFlatten for RenameAllLower {
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
pub struct IndividualRename {
    #[dynamic(rename = "firstName")]
    first_name: String,
    #[dynamic(rename = "lastName")]
    last_name: String,
    age: i32,
}
impl ::cel::types::dynamic::DynamicType for IndividualRename {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(3usize);
        ::cel::types::dynamic::DynamicFlatten::materialize_into(self, &mut m);
        ::cel::Value::Map(::cel::objects::MapValue::Borrow(m))
    }
    fn field(&self, field: &str) -> ::core::option::Option<::cel::Value<'_>> {
        match field {
            "firstName" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.first_name),
                )
            }
            "lastName" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.last_name),
                )
            }
            "age" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.age),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for IndividualRename {
    fn materialize_into<'__cel_a>(
        &'__cel_a self,
        __cel_map: &mut ::vector_map::VecMap<
            ::cel::objects::KeyRef<'__cel_a>,
            ::cel::Value<'__cel_a>,
        >,
    ) {
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("firstName"),
                ::cel::types::dynamic::maybe_materialize(&self.first_name),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("lastName"),
                ::cel::types::dynamic::maybe_materialize(&self.last_name),
            );
        __cel_map
            .insert(
                ::cel::objects::KeyRef::from("age"),
                ::cel::types::dynamic::maybe_materialize(&self.age),
            );
    }
}
fn is_none<T>(opt: &Option<T>) -> bool {
    opt.is_none()
}
pub struct SerdeBasic {
    required: String,
    #[serde(skip)]
    internal_id: u64,
    #[serde(rename = "custom_name")]
    original_name: String,
}
impl ::cel::types::dynamic::DynamicType for SerdeBasic {
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
impl ::cel::types::dynamic::DynamicFlatten for SerdeBasic {
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
pub struct SerdeMultiArgs {
    #[serde(rename = "apiKey", skip_serializing_if = "is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "is_none", rename = "userId")]
    user_id: Option<i32>,
    normal_field: String,
}
impl ::cel::types::dynamic::DynamicType for SerdeMultiArgs {
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
impl ::cel::types::dynamic::DynamicFlatten for SerdeMultiArgs {
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
#[serde(rename_all = "camelCase")]
pub struct SerdeRenameAll {
    user_name: String,
    user_id: i32,
}
impl ::cel::types::dynamic::DynamicType for SerdeRenameAll {
    fn materialize(&self) -> ::cel::Value<'_> {
        let mut m = ::vector_map::VecMap::with_capacity(2usize);
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
            "userId" => {
                ::core::option::Option::Some(
                    ::cel::types::dynamic::maybe_materialize(&self.user_id),
                )
            }
            _ => ::core::option::Option::None,
        }
    }
}
impl ::cel::types::dynamic::DynamicFlatten for SerdeRenameAll {
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
                ::cel::objects::KeyRef::from("userId"),
                ::cel::types::dynamic::maybe_materialize(&self.user_id),
            );
    }
}
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
