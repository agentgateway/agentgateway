FROM docker.io/library/buildpack-deps:trixie
# Pinned Debian package versions break on every point release.
# hadolint ignore=DL3008
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git make jq python3 \
    && rm -rf /var/lib/apt/lists/*
ENV PATH=/root/.cargo/bin:/usr/local/go/bin:/root/go/bin:$PATH \
    CARGO_TERM_COLOR=never
# Install the Go and Rust toolchains that go.mod and rust-toolchain.toml pin.
COPY go.mod rust-toolchain.toml /tmp/pins/
RUN GO_VERSION="$(sed -n 's/^toolchain go//p' /tmp/pins/go.mod)" \
    && test -n "$GO_VERSION" \
    && curl -fsSL "https://go.dev/dl/go${GO_VERSION}.linux-$(dpkg --print-architecture).tar.gz" | tar -C /usr/local -xz
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none \
    && cd /tmp/pins && rustup toolchain install \
    && rustup default "$(rustup show active-toolchain | cut -d' ' -f1)"
COPY . /src
WORKDIR /src
# Data plane (Rust). A dev build keeps debug info and overflow checks; the embedded web UI (the `ui` feature) is left
# out, as it needs a Node build and only serves the admin interface.
RUN cargo fetch --locked && cargo build --locked \
    && ln -s /src/target/debug/agentgateway /usr/local/bin/agentgateway
# Build the tests now so they run offline without recompiling. Tests that need Keycloak or the network are expected to
# fail or skip.
RUN cargo test --locked --tests --no-run \
    && (cargo test --locked --tests || echo "WARNING: cargo test reported failures")
# Control plane (Go). Fetch every module the build, the tests and the tools/ module (helm, golangci-lint) need.
RUN go mod download && go -C tools mod download \
    && go build -trimpath -tags=disable_pgv,agent -o /usr/local/bin/agentgateway-controller ./controller/cmd/agentgateway \
    && go build -o /usr/local/bin/agctl ./controller/cmd/agctl
# Build the tests now so they run offline without recompiling. E2E tests (-tags e2e) need a cluster and are left out.
RUN go test -count=1 -run '^$' ./... \
    && (go test -short -timeout 20m ./... || echo "WARNING: go test reported failures")
