# AGENTS.md

Setup, build and contribution workflow: [DEVELOPMENT.md](DEVELOPMENT.md), [CONTRIBUTION.md](CONTRIBUTION.md).
Architecture (start with configuration): [architecture/](architecture/README.md).

## Where things live
- `crates/agentgateway` – the proxy (Rust); most changes land here.
- `controller/` – Kubernetes controller (Go), with its own Makefile.
- `ui/` – admin UI (pnpm).
- `examples/` – runnable configs, indexed in `examples/README.md`.
- `schema/`, `catalog/model-catalog.json` – generated; do not hand-edit.

## Generated code
- After changing config types or protos, run `make gen`. CI fails on a dirty tree (`check-clean-repo`).
- Controller codegen: `make -C controller verify`.
- Model catalog: edit `catalog/model-catalog-overrides.yaml`, then `make refresh-model-catalog`.

## Checks CI runs
- Rust: `make lint`, `make test`
- UI: `pnpm lint` (in `ui/`)
- Controller: `make -C controller analyze`, `go test ./...`

## Rules the code can't tell you
- Fix root causes; don't add one-off special cases for a single reported issue.
- Never read the `Host` header; use the normalized URI authority.
- Upstream 4xx/5xx are not proxy errors; don't log them at warn/error.
- Follow OpenTelemetry semantic conventions for metric and span names.
- Build TLS configs through `agentgateway::crypto::tls`; clippy rejects direct `rustls` builders.
- Config types use `#[serde(rename_all = "camelCase", deny_unknown_fields)]`; their doc comments become user-facing docs.
