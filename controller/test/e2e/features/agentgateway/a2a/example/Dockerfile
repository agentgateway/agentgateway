# Ultra-minimal A2A Rust test server
FROM rust:1.82-alpine AS builder

# Install build dependencies including OpenSSL
RUN apk add --no-cache musl-dev openssl-dev pkgconfig

# Set work directory
WORKDIR /opt/app-root/

# Copy source code
COPY . .

# Copy the server-only example file
COPY ../http_server_only.rs a2a-rs/examples/http_server_only.rs

# Add the modified example to Cargo.toml
RUN echo '[[example]]' >> a2a-rs/Cargo.toml && \
    echo 'name = "http_server_only"' >> a2a-rs/Cargo.toml && \
    echo 'path = "examples/http_server_only.rs"' >> a2a-rs/Cargo.toml && \
    echo 'required-features = ["http-server"]' >> a2a-rs/Cargo.toml

# Build the modified example from workspace root
RUN cargo build --release --package a2a-rs --example http_server_only --features "http-server" --target x86_64-unknown-linux-musl

# Final stage - minimal runtime
FROM alpine:3.19

# Install only runtime dependencies
RUN apk add --no-cache ca-certificates

# Create non-root user
RUN adduser -D -s /bin/sh appuser

# Copy the binary from builder stage
COPY --from=builder /opt/app-root/target/x86_64-unknown-linux-musl/release/examples/http_server_only /usr/local/bin/a2a-server

# Fix permissions
RUN chown appuser:appuser /usr/local/bin/a2a-server && \
    chmod +x /usr/local/bin/a2a-server

# Expose port
EXPOSE 9999

USER appuser

# Run the server (it will run client tests then keep server running)
CMD ["/usr/local/bin/a2a-server"]
