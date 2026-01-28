use cel_derive::DynamicType;

#[derive(DynamicType)]
pub enum BackendProtocol {
    #[dynamic(rename = "http")]
    Http,
    #[dynamic(rename = "grpc")]
    Grpc,
    #[dynamic(rename = "ws")]
    WebSocket,
}
