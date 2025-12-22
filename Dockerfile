ARG BUILDER=base

FROM --platform=$BUILDPLATFORM docker.io/library/node:23.11.0-bookworm AS node

WORKDIR /app

COPY ui .

RUN --mount=type=cache,target=/app/npm/cache npm install

RUN --mount=type=cache,target=/app/npm/cache npm run build

FROM --platform=linux/arm64 docker.io/library/rust:1.91.1-trixie AS arm64-sysroot
FROM --platform=linux/arm64 docker.io/library/rust:1.91.1-slim-bookworm AS arm64-musl-sysroot
FROM docker.io/library/rust:1.91.1-slim-bookworm AS amd64-musl-sysroot

FROM --platform=$BUILDPLATFORM docker.io/library/rust:1.91.1-trixie AS base-builder

RUN apt-get update && apt-get -y install clang-17 clang++-17 lld-17

RUN rustup target add aarch64-unknown-linux-musl \
    x86_64-unknown-linux-musl \
    aarch64-unknown-linux-gnu \
    x86_64-unknown-linux-gnu

FROM --platform=$BUILDPLATFORM base-builder AS builder
ARG TARGETARCH
ARG BUILDER
ARG PROFILE=release
ARG VERSION
ARG GIT_REVISION

COPY --from=arm64-sysroot /lib /sysroots/arm64/lib/
COPY --from=arm64-sysroot /usr/include /sysroots/arm64/usr/include/
COPY --from=arm64-sysroot /usr/lib /sysroots/arm64/usr/lib/

COPY --from=arm64-musl-sysroot /lib /sysroots/arm64-musl/lib/
COPY --from=arm64-musl-sysroot /usr/include /sysroots/arm64-musl/usr/include/
COPY --from=arm64-musl-sysroot /usr/lib /sysroots/arm64-musl/usr/lib/

COPY --from=amd64-musl-sysroot /lib /sysroots/amd64-musl/lib/
COPY --from=amd64-musl-sysroot /usr/include /sysroots/amd64-musl/usr/include/
COPY --from=amd64-musl-sysroot /usr/lib /sysroots/amd64-musl/usr/lib/

RUN <<EOF
mkdir /build
if [ "$TARGETARCH" = "arm64" ]; then
  if [ "$BUILDER" = "musl" ]; then
    echo aarch64-unknown-linux-musl > /build/target
    ln -s /sysroots/arm64-musl /sysroots/current
  else
    echo aarch64-unknown-linux-gnu > /build/target
    ln -s /sysroots/arm64 /sysroots/current
  fi
else
  if [ "$BUILDER" = "musl" ]; then
    echo x86_64-unknown-linux-musl > /build/target
    ln -s /sysroots/amd64-musl /sysroots/current
  else
    echo x86_64-unknown-linux-gnu > /build/target
    ln -s / /sysroots/current
  fi
fi
echo "Building $(cat /build/target)"
EOF

WORKDIR /app

COPY Makefile Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY cargo-docker.config.toml ./
RUN cat cargo-docker.config.toml >> .cargo/config.toml

COPY crates ./crates
COPY common ./common
COPY --from=node /app/out ./ui/out

ENV CC=clang
ENV CXX=clang++
ENV LDFLAGS="-fuse-ld=lld-17"
ENV CFLAGS="--sysroot=/sysroots/current -fuse-ld=lld-17"

RUN \
    --mount=type=cache,id=cargo,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    cargo fetch --locked
RUN --mount=type=cache,target=/app/target \
    --mount=type=cache,id=cargo,target=/usr/local/cargo/registry  \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    <<EOF
export VERSION="${VERSION}"
export GIT_REVISION="${GIT_REVISION}"
if [ "$BUILDER" = "musl" ]; then
  export CFLAGS="${CFLAGS} -static"
fi
cargo build --features ui --target "$(cat /build/target)" --profile ${PROFILE} || exit 1
mkdir /out
mv /app/target/$(cat /build/target)/${PROFILE}/agentgateway /out
EOF

RUN <<EOF
# only check version in amd64 builds (i.e. native builds)
if [ "$TARGETARCH" = "amd64" ]; then
  /out/agentgateway --version
  # Fail if version is not set
  if /out/agentgateway --version | grep -q '"unknown"'; then
    exit 1
  fi
fi
EOF

FROM cgr.dev/chainguard/glibc-dynamic AS runner

WORKDIR /

COPY --from=builder /out/agentgateway /app/agentgateway

LABEL org.opencontainers.image.source=https://github.com/agentgateway/agentgateway
LABEL org.opencontainers.image.description="Agentgateway is an open source project that is built on AI-native protocols to connect, secure, and observe agent-to-agent and agent-to-tool communication across any agent framework and environment."

ENTRYPOINT ["/app/agentgateway"]
