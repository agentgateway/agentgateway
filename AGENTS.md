# AGENTS.md

Setup, build and contribution workflow: [DEVELOPMENT.md](DEVELOPMENT.md), [CONTRIBUTION.md](CONTRIBUTION.md).
Architecture (start with configuration): [architecture/](architecture/README.md).

Commits need a DCO sign-off (`git commit -s`). Fill in the [PR template](.github/pull_request_template.md) when opening a PR.

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
The full set is defined in [.github/workflows/pull_request.yml](.github/workflows/pull_request.yml). It also covers race detection, E2E tests, codegen and Go module checks that the list below omits.
- Rust: `make lint`, `make check-default-members`, `make test`
- UI: `pnpm lint` (in `ui/`)
- Controller: `make -C controller analyze`, `go test ./...`

## Rules the code can't tell you
- Fix root causes; don't add one-off special cases for a single reported issue.
- Never read the `Host` header; use the normalized URI authority.
- Upstream 4xx/5xx are not proxy errors; don't log them at warn/error.
- Follow OpenTelemetry semantic conventions for metric and span names.
- Build TLS configs with `builder_with_provider`, using a provider from `agentgateway::crypto::tls`; clippy rejects direct `rustls` builders. Use `provider()` for defaults and `provider_with_options_validated` for user-configured cipher suites or key exchange groups, which keeps the FIPS checks in place.
- New config types use `#[serde(rename_all = "camelCase", deny_unknown_fields)]`; their doc comments become user-facing docs.
