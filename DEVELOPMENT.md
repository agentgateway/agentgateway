# Development

`make` is the public contributor entrypoint for this repository.

## Which Command Should I Run?

| Goal | Command | What it does |
| --- | --- | --- |
| Fast local verification | `make test` | Runs the default proxy, controller, and UI test path without creating a cluster. |
| Shared Kubernetes env without Tilt | `make env-up` | Creates the shared kind env, reconciles add-ons, builds images, and deploys the Helm release. |
| Daily Kubernetes inner loop | `make dev` | Starts Tilt. Tilt shows `env-cluster`, `env-addons`, and `env-ready` as explicit bootstrap resources before live-updated app resources. |
| Cluster-backed end-to-end validation | `make e2e` | Brings up the supported e2e profile and runs the local e2e suite against it. |

Start here on a fresh machine:

```bash
make help
make bootstrap
make doctor
make test
```

## Prerequisites

Host tools checked by `make doctor`:

- Rust / Cargo
- Go
- Node.js / npm
- Docker
- kubectl
- Helm
- kind
- ctlptl
- jq
- Python 3
- Tilt (optional, only for `make dev`)

`make bootstrap` installs repo-managed dependencies and UI packages. It does not install host tooling for you.

## Fast Local Loop

Use the root targets when you want the default repo workflow:

```bash
make lint
make fix
make test
```

The default contracts are:

- `make lint` runs check-only validation.
- `make fix` runs mutating format and auto-fix flows.
- `make test` stays local and fast by default. It does not create a cluster.

## Shared Kubernetes Environment

The shared environment is managed through `ctlptl` plus [tools/dev-env.sh](tools/dev-env.sh).

```bash
make env-up
make env-status
make env-down
```

`make env-status` now checks a real readiness contract, not just cluster existence. Ready means:

- the configured kind context exists
- the configured registry is running on the expected host port
- required namespaces and add-ons are present

Profiles are available for specialized flows:

```bash
make env-up PROFILE=e2e
make env-up PROFILE=ci
```

If the default registry port is already in use on your machine, override it explicitly:

```bash
REGISTRY_PORT=5001 REGISTRY_HOST=localhost:5001 make env-up
```

## Daily Tilt Inner Loop

`make dev` is the preferred Kubernetes development loop:

```bash
make dev
```

You can still run Tilt directly:

```bash
tilt up
```

Tilt now exposes env bootstrap as explicit resources:

- `env-cluster`
- `env-addons`
- `env-ready`

That keeps bootstrap failures visible and retryable from the Tilt UI instead of hiding them in Tiltfile load-time side effects.

Use `make env-up` when you want the shared env and Helm release without Tilt. Use `make dev` when you want live rebuilds and the Tilt UI.

If you need a non-default registry port for Tilt as well, pass the override into the process environment:

```bash
REGISTRY_PORT=5001 REGISTRY_HOST=localhost:5001 make dev
REGISTRY_PORT=5001 REGISTRY_HOST=localhost:5001 tilt up
```

## End-To-End Workflow

Run the supported local e2e path with:

```bash
make e2e
```

Common overrides:

```bash
make e2e E2E_PROFILE=ci
make e2e E2E_TEST_PATTERN='^TestAgentgatewayIntegration'
```

## Subsystem Workflows

Subsystem-native commands remain valid when you are touching one area only.

### Proxy

```bash
make test-proxy
make lint-proxy
cargo test --all-targets
```

### Controller

```bash
make test-controller
make lint-controller
make -C controller help
make -C controller unit
```

### UI

```bash
make test-ui
make lint-ui
npm --prefix ui run verify
npm --prefix ui run fix
```

## Compatibility Notes

Older controller bootstrap targets still exist as compatibility wrappers:

- `make -C controller setup-base`
- `make -C controller setup`
- `make -C controller run`

New contributor docs and automation should prefer the root commands instead.
