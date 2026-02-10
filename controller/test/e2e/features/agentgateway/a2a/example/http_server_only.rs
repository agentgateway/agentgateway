//! A server-only HTTP example that runs continuously

use a2a_rs::adapter::{
    DefaultRequestProcessor, HttpServer,
    InMemoryTaskStorage, NoopPushNotificationSender, SimpleAgentInfo, BearerTokenAuthenticator,
};

mod common;
use common::SimpleAgentHandler;
use a2a_rs::observability;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better observability
    observability::init_tracing();

    println!("🚀 Starting A2A HTTP Server");
    println!("==========================");

    // Run the server (this will run indefinitely)
    run_server().await.expect("Server failed");
    
    Ok(())
}

async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    println!("🌐 Starting HTTP server...");

    // Create server components
    let push_sender = NoopPushNotificationSender;
    let storage = InMemoryTaskStorage::with_push_sender(push_sender);
    let handler = SimpleAgentHandler::with_storage(storage);
    let processor = DefaultRequestProcessor::with_handler(handler);

    // Create agent info
    let agent_info = SimpleAgentInfo::new(
        "Example A2A Agent".to_string(),
        "http://localhost:9999".to_string(),
    )
    .with_description("An example A2A agent using the a2a-protocol crate".to_string())
    .with_provider(
        "Example Organization".to_string(),
        "https://example.org".to_string(),
    )
    .with_documentation_url("https://example.org/docs".to_string())
    .with_streaming()
    .add_comprehensive_skill(
        "echo".to_string(),
        "Echo Skill".to_string(),
        Some("Echoes back the user's message".to_string()),
        Some(vec!["echo".to_string(), "respond".to_string()]),
        Some(vec!["Echo: Hello World".to_string()]),
        Some(vec!["text".to_string()]),
        Some(vec!["text".to_string()]),
    );

    // Server with bearer token authentication
    let tokens = vec!["secret-token".to_string()];
    let authenticator = BearerTokenAuthenticator::new(tokens);
    let server = HttpServer::with_auth(
        processor,
        agent_info,
        "0.0.0.0:9999".to_string(),
        authenticator,
    );

    println!("🔗 HTTP server listening on http://0.0.0.0:9999");
    println!("✅ Server is running and ready to accept requests");
    server.start().await.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}
