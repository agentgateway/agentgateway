# Quickstart (GitHub, no local install)

1. Click **Code → Create codespace on main**.
2. In the terminal:
   cargo fmt --all
   cargo clippy --all -- -D warnings
   cargo test --all
3. If you touched the UI:
   cd ui
   corepack enable
   pnpm install --frozen-lockfile
   pnpm test:e2e

# Local Development

This page contains instructions on how to run everything locally.

## Build from Source

Requirements:
- Rust 1.86+
- Node 24.17.0 (see `ui/.nvmrc`)

The UI is built with [pnpm](https://pnpm.io/). The version is pinned in
`ui/package.json` under `packageManager`, and Corepack (bundled with Node)
installs it for you. Run `corepack enable` once per environment.

Build the agentgateway UI:

```bash
cd ui
pnpm install --frozen-lockfile
pnpm build
```

`--frozen-lockfile` installs exactly what `ui/pnpm-lock.yaml` records and fails
if it disagrees with `package.json`. CI, the release build and the Docker images
run the same command. CI runs `pnpm lint`, `pnpm test:e2e`, and
`pnpm test:visual-regressions`. `pnpm dev` starts the dev server on
http://localhost:19000.

Dependencies are pinned to exact versions. To change one, edit
`ui/package.json`, run `pnpm install` without `--frozen-lockfile`, and commit
the updated lockfile.

Both test commands invoke Playwright directly, with configurations under
`ui/tests/`. CI runs both suites in the pinned Linux Playwright image.
Local end-to-end tests run directly with `pnpm test:e2e`, without a container.
For local screenshot comparisons and baseline updates, use the explicit
Linux container launcher from `ui/`:

```bash
pnpm test:visual-regressions:linux
pnpm test:visual-regressions:linux --grep 'LLM Models'
pnpm test:visual-regressions:linux --update-snapshots=all
```

Docker must be running. To use Podman instead, run
`DOCKER_BUILDER=podman pnpm test:visual-regressions:linux`.
The launcher selects the repository's Node and pnpm versions, then calls
`pnpm test:visual-regressions` inside the container. Running the direct command
on macOS can produce different pixels because rendering depends on the platform.
Keep baselines unchanged for appearance-preserving refactors. After intentional
visual changes, inspect every updated PNG under
`ui/tests/visual-regression/baselines/` and rerun the comparison without the update flag.

Generated traces and screenshot comparisons are written to `ui/tests/test-results/`.
The HTML report is written to `ui/tests/playwright-report/`. Both directories are
ignored by Git. From `ui/`, open the report with:

```bash
pnpm exec playwright show-report tests/playwright-report
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

On Apple Silicon Macs, install the Linux x86_64 variant of the repo's active Rust
toolchain so `cross` can run Rust inside that container:

```bash
rust_version=$(awk -F'"' '/^channel =/ {print $2}' rust-toolchain.toml)
rustup toolchain install "${rust_version}-x86_64-unknown-linux-gnu" \
   --profile minimal \
   --force-non-host
```

Create the local Kind cluster and registry if they do not already exist:

```bash
ctlptl create cluster kind --name kind-kind --registry=ctlptl-registry
```

Run:

```bash
tilt up
```
