FROM docker.io/library/node:23.11.0-bookworm AS node

WORKDIR /app

COPY ui .

RUN npm install

RUN npm run build

FROM docker.io/library/rust:1.88.0-slim-bookworm AS builder

ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler make libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Makefile Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY common ./common
COPY --from=node /app/out ./ui/out
RUN --mount=type=cache,id=cargo,target=/usr/local/cargo/registry cargo fetch --locked
RUN --mount=type=cache,target=/app/target --mount=type=cache,id=cargo,target=/usr/local/cargo/registry make build &&  \
   mkdir /out && \
    mv /app/target/release/agentgateway /out

FROM gcr.io/distroless/cc-debian12 AS runner 

ARG TARGETARCH
WORKDIR /app

COPY --from=builder /out/agentgateway /app/agentgateway

LABEL org.opencontainers.image.source=https://github.com/agentgateway/agentgateway
LABEL org.opencontainers.image.description="Agentgateway is an open source project that is built on AI-native protocols to connect, secure, and observe agent-to-agent and agent-to-tool communication across any agent framework and environment."

ENTRYPOINT ["/app/agentgateway"]
