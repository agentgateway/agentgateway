use cel_derive::DynamicType;

#[derive(DynamicType)]
pub enum BackendType {
    Http,
    Grpc,
    WebSocket,
}
