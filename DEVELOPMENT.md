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
run the same command. CI runs `pnpm lint` and `pnpm test:e2e`.
`pnpm dev` starts the dev server on http://localhost:19000.

Dependencies are pinned to exact versions. To change one, edit
`ui/package.json`, run `pnpm install` without `--frozen-lockfile`, and commit
the updated lockfile.

### Local Visual Comparisons

Visual comparisons are optional local checks. They do not run in CI, and
expected screenshots are generated from a Git revision for each comparison.
From `ui/`, run:

```sh
pnpm test:visual-regressions
pnpm test:visual-regressions --base upstream/main
pnpm test:visual-regressions --base main --video --gif
```

The default reference is local `main`. Fetch or update that ref first when you
want newer upstream changes. The command prints the resolved commit and captures
the current working tree, including staged, unstaged and nonignored new UI/schema
files. It builds both versions independently and runs the same Playwright cases,
fixtures and screenshot rules against both, in a digest-pinned Linux image.
It does not switch branches or modify your index or application source.

Docker must be running. Use `DOCKER_BUILDER=podman` to select Podman. Optional
MP4/GIF exports require FFmpeg on the host; HTML and image comparisons work
without it. An explicit `NPM_CONFIG_USERCONFIG` can be mounted read-only for
container package access. Local package configuration is excluded from source
snapshots and reports.

The container runs package setup and capture as the owner of the mounted source
directory. This keeps generated files writable by the caller on native Linux
and respects the ownership mapping used by rootless engines.

Each run prints an ignored output directory under
`ui/tests/visual-regression/results.local/`. Open its `index.html` for the
side-by-side gallery. Passing cases have before/after images too; failures retain
diff images and Playwright diagnostics. Optional media presents those captured
comparisons for manual attachment to a PR. The capture coverage and source
identity are recorded in `results.json`; per-stage seconds are in `timings.json`,
and available container CPU/peak-memory counters are in `resources.json`.

Exit code 0 means a complete matching comparison, 1 means the comparison failed,
and 2 means setup, capture, report or export could not complete. Reports and
available evidence remain after failures. The reference is never automatically
updated to accept a candidate image. A chosen reference that lacks a captured
page is an incomplete comparison, not a matching result.

Package downloads are cached between runs. Both applications are rebuilt and
both image sets are captured on every run. The runner uses one browser worker to
limit memory pressure. Disposable source copies and their dependency trees are
removed when the command finishes; result directories remain available to review.

Functional E2E tests still run directly with `pnpm test:e2e`. The local runner's
helper tests use `node --test tests/visual-regression/*.test.ts` and require the
same local Chromium installation as the functional E2E tests.
The separate Linux ownership check uses
`node --test tests/visual-regression/ownership.linux.ts`. It requires a rootful
Docker or Podman engine, its cached renderer image, and a nonroot caller. It
checks success/failure exit codes, cache ownership and host cleanup, and is
skipped on other platforms and for root callers.

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
