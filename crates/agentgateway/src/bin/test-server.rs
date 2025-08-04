//! Simple HTTP test server for benchmarking
//! 
//! This server provides various endpoints for testing AgentGateway proxy performance.

use std::env;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let listen_addr = if args.len() > 1 {
        args[1].clone()
    } else {
        "127.0.0.1:3001".to_string()
    };

    let addr: SocketAddr = listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;
    
    println!("Test server listening on {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle_connection(stream));
    }
}

async fn handle_connection(mut stream: TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut buffer = [0; 4096];
    let bytes_read = stream.read(&mut buffer).await?;
    
    if bytes_read == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let lines: Vec<&str> = request.lines().collect();
    
    if lines.is_empty() {
        return Ok(());
    }

    let request_line = lines[0];
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    
    if parts.len() < 2 {
        return Ok(());
    }

    let method = parts[0];
    let path = parts[1];

    // Route handling
    let (status, content_type, body) = match (method, path) {
        ("GET", "/") => {
            ("200 OK", "text/plain", "Hello from test server!")
        }
        ("GET", "/test") => {
            ("200 OK", "application/json", r#"{"message": "test response", "timestamp": 1234567890}"#)
        }
        ("GET", "/warmup") => {
            ("200 OK", "text/plain", "warmup")
        }
        ("GET", path) if path.starts_with("/payload") => {
            // For simplicity, return a fixed 1KB payload
            ("200 OK", "text/plain", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx")
        }
        ("GET", "/health") => {
            ("200 OK", "application/json", r#"{"status": "healthy", "uptime": 123}"#)
        }
        ("GET", "/json") => {
            ("200 OK", "application/json", r#"{"message": "Hello, World!"}"#)
        }
        ("GET", "/plaintext") => {
            ("200 OK", "text/plain", "Hello, World!")
        }
        _ => {
            ("404 Not Found", "text/plain", "Not Found")
        }
    };

    // Build HTTP response
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: keep-alive\r\nServer: test-server\r\nDate: {}\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        httpdate::fmt_http_date(std::time::SystemTime::now()),
        body
    );

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}
