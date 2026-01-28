use cel_derive::DynamicType;
pub enum BackendProtocol {
    #[dynamic(rename = "http")]
    Http,
    #[dynamic(rename = "grpc")]
    Grpc,
    #[dynamic(rename = "ws")]
    WebSocket,
}
impl ::cel::types::dynamic::DynamicType for BackendProtocol {
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
