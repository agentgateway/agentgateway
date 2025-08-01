# Dockerfile for running AgentGateway benchmarks with multi-process support
FROM rust:1.88-slim

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    curl \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy the entire project
COPY . .

# Build the project with benchmark features
RUN cargo build --release --package agentgateway --features internal_benches

# Build the test server binary
RUN cargo build --release --package agentgateway --bin test-server

# Build the agentgateway binary
RUN cargo build --release --package agentgateway-app

# Create benchmark reports directory
RUN mkdir -p target/benchmark_reports

# Set environment variables for benchmarking
ENV RUST_LOG=error
ENV CARGO_TARGET_DIR=/app/target

# Expose ports for benchmarking (test server and proxy)
EXPOSE 3001 8080

# Default command runs all benchmarks
CMD ["cargo", "bench", "--package", "agentgateway", "--features", "internal_benches"]
