# Quickstart (GitHub, no local install)

1. Click **Code → Create codespace on main**.
2. In the terminal:
   cargo fmt --all
   cargo clippy --all -- -D warnings
   cargo test --all
3. If you touched the UI:
   cd ui
   npm ci
   npm test

# Local Development

This page contains instructions on how to run everything locally.

## Build from Source

Requirements:
- Rust 1.86+
- npm 10+

Build the agentgateway UI:

```bash
cd ui
npm install
npm run build
```

Build the agentgateway binary:

```bash
cd ..
export CARGO_NET_GIT_FETCH_WITH_CLI=true
make build
```

Run the agentgateway binary:

```bash
./target/release/agentgateway
```
Open your browser and navigate to `http://localhost:15000/ui` to see the agentgateway UI.

## Local Development with Tilt (Kubernetes)

For developing against a local Kind cluster with live reloading:

Requirements (in addition to the above):
- [Kind](https://kind.sigs.k8s.io/)
- [Tilt](https://tilt.dev/)
- [ctlptl](https://github.com/tilt-dev/ctlptl) - used to create a Kind cluster with a local registry
- [cross](https://github.com/cross-rs/cross) - required for ensuring the Rust backend compiles (or cross-compiles) for Linux
- Docker (or Podman) — required by both Kind and `cross`
- Go 1.22+ (for the controller)

>NOTE: On Apple Silicon Macs, Tilt runs the `cross` build container as `linux/amd64` because the
>default `cross` image for `aarch64-unknown-linux-gnu` does not publish a `linux/arm64` manifest.
>This still produces the Linux arm64 dataplane binary used by Kind.

Create the local Kind cluster and registry if they do not already exist:

```bash
ctlptl create cluster kind --name kind-kind --registry=ctlptl-registry
```

Run:

```bash
tilt up
```
