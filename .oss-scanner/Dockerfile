FROM docker.io/library/rust:1.99.0-trixie
ARG GO_VERSION=1.27.1
# Pinned Debian package versions break on every point release.
# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git make jq python3 \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" | tar -C /usr/local -xz
ENV PATH=/usr/local/go/bin:/root/go/bin:$PATH \
    CARGO_TERM_COLOR=never
COPY . /src
WORKDIR /src
# rust-toolchain.toml pins the toolchain; make sure it is installed now, while online.
RUN rustup show active-toolchain || rustup toolchain install
# Data plane (Rust). A dev build keeps debug info and overflow checks; the embedded web UI (the `ui` feature) is left
# out, as it needs a Node build and only serves the admin interface.
RUN cargo fetch --locked && cargo build --locked \
    && ln -s /src/target/debug/agentgateway /usr/local/bin/agentgateway
# Build the tests now so they run offline without recompiling. Tests that need Keycloak or the network are expected to
# fail or skip.
RUN cargo test --locked --tests --no-run \
    && (cargo test --locked --tests || echo "WARNING: cargo test reported failures")
# Control plane (Go). Fetch every module the build and tests need.
RUN go mod download \
    && go build -trimpath -tags=disable_pgv,agent -o /usr/local/bin/agentgateway-controller ./controller/cmd/agentgateway \
    && go build -o /usr/local/bin/agctl ./controller/cmd/agctl
RUN go test -short -timeout 20m ./... || echo "WARNING: go test reported failures"
