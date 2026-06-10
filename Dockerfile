# syntax=docker/dockerfile:1.11
#
# Multi-architecture build for linux/amd64, linux/arm64, linux/s390x.
#
# Build with buildx (TARGETARCH/BUILDARCH are set automatically per --platform):
#
#   docker buildx build -f Dockerfile.multiarch \
#     --platform linux/amd64,linux/arm64,linux/s390x \
#     --build-arg VERSION="$(git describe --tags --always --dirty)" \
#     --build-arg GIT_REVISION="$(git rev-parse HEAD)" \
#     -t <repo>/agentgateway:<tag> --push .
#
# Do NOT pass --build-arg TARGETARCH; buildx populates it per platform.
#
# Key design notes:
#  * The UI is built ONCE on the build host (its output is static and
#    architecture-independent) and shared by every target arch.
#  * lightningcss / @tailwindcss/oxide ship no s390x prebuilt natives, so we
#    compile them from source — but ONLY when the build host is s390x
#    (keyed on BUILDARCH). On amd64/arm64 hosts npm's prebuilt natives are used
#    and these heavy stages are skipped entirely by BuildKit.
#  * tokio_unstable + force-frame-pointers come from .cargo/config.toml (copied
#    into the build); we do NOT re-specify them via RUSTFLAGS here.

ARG BUILDER=base

# ─────────────────────────────────────────────────────────────────────────────
# UI native deps for s390x — only materialized when BUILDARCH=s390x.
# These run on the build host ($BUILDPLATFORM) since they feed the UI build.
# ─────────────────────────────────────────────────────────────────────────────
FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.96.0-bookworm AS lightningcss-s390x

COPY --from=docker.io/library/node:23.11.0-bookworm /usr/local/bin/node /usr/local/bin/node
COPY --from=docker.io/library/node:23.11.0-bookworm /usr/local/include/node /usr/local/include/node
COPY --from=docker.io/library/node:23.11.0-bookworm /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -sf /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -sf /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 make g++ git \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g @napi-rs/cli

# Keep in sync with the lightningcss version resolved in ui/package-lock.json.
ARG LIGHTNINGCSS_VERSION=1.30.2
RUN rustup target add s390x-unknown-linux-gnu
RUN git clone --depth 1 --branch v${LIGHTNINGCSS_VERSION} \
    https://github.com/parcel-bundler/lightningcss.git /lightningcss-src
WORKDIR /lightningcss-src
RUN npm install
RUN napi build --platform --release --target s390x-unknown-linux-gnu \
    --manifest-path /lightningcss-src/node/Cargo.toml \
    --output-dir /lightningcss-src/node
RUN ls -la /lightningcss-src/node/*.node

FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.96.0-bookworm AS oxide-s390x

COPY --from=docker.io/library/node:23.11.0-bookworm /usr/local/bin/node /usr/local/bin/node
COPY --from=docker.io/library/node:23.11.0-bookworm /usr/local/include/node /usr/local/include/node
COPY --from=docker.io/library/node:23.11.0-bookworm /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -sf /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -sf /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx

RUN apt-get update && apt-get install -y --no-install-recommends \
    python3 make g++ git \
    && rm -rf /var/lib/apt/lists/*

RUN npm install -g @napi-rs/cli pnpm

# Keep in sync with the @tailwindcss/oxide version resolved in ui/package-lock.json.
ARG OXIDE_VERSION=4.1.18
RUN rustup target add s390x-unknown-linux-gnu
RUN git clone --depth 1 --branch v${OXIDE_VERSION} \
    https://github.com/tailwindlabs/tailwindcss.git /oxide-src
WORKDIR /oxide-src
RUN pnpm install --filter @tailwindcss/node --ignore-scripts
WORKDIR /oxide-src/crates/node
RUN napi build --platform --release --target s390x-unknown-linux-gnu \
    --output-dir /oxide-src/crates/node
RUN ls -la /oxide-src/crates/node/*.node

# ─────────────────────────────────────────────────────────────────────────────
# Per-build-host native bundle. BuildKit only builds the variant matching
# BUILDARCH, so the s390x source-builds above are skipped on amd64/arm64 hosts.
# ─────────────────────────────────────────────────────────────────────────────
FROM busybox AS ui-natives-s390x
COPY --from=lightningcss-s390x \
    /lightningcss-src/node/lightningcss.linux-s390x-gnu.node /natives/
COPY --from=oxide-s390x \
    /oxide-src/crates/node/tailwindcss-oxide.linux-s390x-gnu.node /natives/

FROM busybox AS ui-natives-amd64
RUN mkdir -p /natives

FROM busybox AS ui-natives-arm64
RUN mkdir -p /natives

FROM ui-natives-${BUILDARCH} AS ui-natives

# ─────────────────────────────────────────────────────────────────────────────
# Stage: build the Next.js UI once, on the build host. Output (out/) is static
# and reused by every target architecture's Rust build.
# ─────────────────────────────────────────────────────────────────────────────
FROM --platform=$BUILDPLATFORM docker.io/library/node:23.11.0-bookworm AS node
WORKDIR /app
COPY ui .
COPY schema /schema

RUN --mount=type=cache,target=/app/npm/cache \
    npm install --cache /app/npm/cache

# Inject host-arch natives if provided (s390x host only; empty otherwise).
COPY --from=ui-natives /natives /tmp/ui-natives
RUN <<EOF
set -eu
if [ -f /tmp/ui-natives/lightningcss.linux-s390x-gnu.node ]; then
  echo "Injecting prebuilt s390x UI natives"
  cp /tmp/ui-natives/lightningcss.linux-s390x-gnu.node \
     node_modules/lightningcss/lightningcss.linux-s390x-gnu.node
  cp /tmp/ui-natives/tailwindcss-oxide.linux-s390x-gnu.node \
     node_modules/@tailwindcss/oxide/tailwindcss-oxide.linux-s390x-gnu.node
fi
EOF

RUN --mount=type=cache,target=/app/npm/cache npm run build

# ─────────────────────────────────────────────────────────────────────────────
# Stage: Rust musl builder (static). s390x has no musl target — guard it.
# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/library/rust:1.96.0-trixie AS musl-builder
ARG TARGETARCH

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && \
    echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' > /etc/apt/apt.conf.d/keep-cache && \
    apt-get update && apt-get install -y --no-install-recommends \
    make musl-tools

RUN <<EOF
set -eu
mkdir /build
case "$TARGETARCH" in
  arm64) rustup target add aarch64-unknown-linux-musl; echo aarch64-unknown-linux-musl > /build/target ;;
  s390x) echo "ERROR: musl builds are unsupported on s390x (no s390x-unknown-linux-musl target). Use BUILDER=base." >&2; exit 1 ;;
  *)     rustup target add x86_64-unknown-linux-musl; echo x86_64-unknown-linux-musl > /build/target ;;
esac
EOF

# ─────────────────────────────────────────────────────────────────────────────
# Stage: Rust glibc builder (default).
# ─────────────────────────────────────────────────────────────────────────────
FROM docker.io/library/rust:1.96.0-bookworm AS base-builder
ARG TARGETARCH

RUN <<EOF
set -eu
mkdir /build
case "$TARGETARCH" in
  arm64) echo aarch64-unknown-linux-gnu > /build/target ;;
  s390x) echo s390x-unknown-linux-gnu  > /build/target ;;
  *)     echo x86_64-unknown-linux-gnu > /build/target ;;
esac
echo "Building $(cat /build/target)"
EOF

# ─────────────────────────────────────────────────────────────────────────────
# Stage: main Rust build (runs on the TARGET platform; emulated when needed).
# ─────────────────────────────────────────────────────────────────────────────
FROM ${BUILDER}-builder AS builder
ARG TARGETARCH
ARG PROFILE=release
ARG VERSION
ARG GIT_REVISION
ARG CARGO_FEATURES=agentgateway/ui
ARG CARGO_NO_DEFAULT_FEATURES=false

WORKDIR /app

COPY Makefile Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
COPY tools ./tools
COPY --from=node /app/out ./ui/out

RUN \
    --mount=type=cache,id=cargo,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    cargo fetch --locked

# NOTE: tokio_unstable + force-frame-pointers (and the frame-pointer CFLAGS) come
# from .cargo/config.toml for all linux targets — intentionally not duplicated here.
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,id=cargo,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    <<EOF
# NOTE: intentionally 'set -e' (not '-eu'): VERSION/GIT_REVISION are optional at
# the shell level; a missing VERSION is caught clearly by the '"unknown"' check
# below rather than crashing with a cryptic "parameter not set".
set -e
export VERSION="${VERSION}"
export GIT_REVISION="${GIT_REVISION}"
# Fail fast — before the multi-minute compile — if the version wasn't provided.
if [ -z "${VERSION}" ]; then
  echo "ERROR: --build-arg VERSION=... is required (also pass --build-arg GIT_REVISION=...)." >&2
  echo "  e.g. --build-arg VERSION=\"\$(git describe --tags --always --dirty)\" --build-arg GIT_REVISION=\"\$(git rev-parse HEAD)\"" >&2
  exit 1
fi
# Build jemalloc for 64KB pages on arm64 so the image runs on hosts with any
# page size <= 64KB (4KB/16KB/64KB).
if [ "${TARGETARCH}" = "arm64" ]; then
  export JEMALLOC_SYS_WITH_LG_PAGE=16
fi
TARGET="$(cat /build/target)"
if [ "${CARGO_NO_DEFAULT_FEATURES}" = "true" ]; then
  cargo build --no-default-features --features "${CARGO_FEATURES}" --target "${TARGET}" --profile ${PROFILE}
else
  cargo build --features "${CARGO_FEATURES}" --target "${TARGET}" --profile ${PROFILE}
fi
mkdir /out
mv /app/target/${TARGET}/${PROFILE}/agentgateway /out/
/out/agentgateway --version
# Fail the build if the version was not stamped in.
if /out/agentgateway --version | grep -q '"unknown"'; then
  echo "ERROR: version is 'unknown' — VERSION/GIT_REVISION not set" >&2
  exit 1
fi
EOF

# ─────────────────────────────────────────────────────────────────────────────
# Stage: final runtime image.
# debian:bookworm-slim is used (not chainguard glibc-dynamic) because it has
# s390x/arm64/amd64 variants; chainguard publishes no s390x image.
# ─────────────────────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runner
ARG TARGETARCH

ENV AGENTGATEWAY_ENV=container

WORKDIR /

COPY --from=builder /out/agentgateway /app/agentgateway

LABEL org.opencontainers.image.source=https://github.com/agentgateway/agentgateway
LABEL org.opencontainers.image.description="Agentgateway is an open source project that is built on AI-native protocols to connect, secure, and observe agent-to-agent and agent-to-tool communication across any agent framework and environment."

ENTRYPOINT ["/app/agentgateway"]
