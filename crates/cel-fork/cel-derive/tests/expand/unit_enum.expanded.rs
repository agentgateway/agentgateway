use cel_derive::DynamicType;
pub enum BackendType {
    Http,
    Grpc,
    WebSocket,
}
impl ::cel::types::dynamic::DynamicType for BackendType {
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
