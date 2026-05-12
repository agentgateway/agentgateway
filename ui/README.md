# 🌐 agentgateway UI

A web UI for agentgateway configuration and management.

## 📁 Project Structure

```
ui/public/
├── cel-schema.json    # The CEL JSON schema (copied for public UI access)
├── config-schema.json # The config JSON schema (copied for public UI access)
├── config.d.ts        # Generated types from config schema
└── cel.d.ts           # Generated types from CEL schema
ui/src/
├── components/        # Reusable UI components & Layout
├── contexts/          # React Context providers (Theme, Server, Loading, Wizard)
├── pages/             # Page components
├── styles/            # Global styles, theme vars, Emotion & Antd config
├── api/               # API client functions
├── config.d.ts        # Generated types from config schema
└── cel.d.ts           # Generated types from CEL schema
```

## ⚡ Quick Start

First, make sure agentgateway is running. The UI dev server runs on port 5173 by default; any config that binds to 5173 should be moved to a free port before starting agentgateway. (The xDS dev server, if you use one, runs on port 5174.)

```bash
# Start the agentgateway. For example:
agentgateway -f ./config.yaml
```

Then run the UI dev server.

```bash
# From the root of the repo:
yarn --cwd=./ui install
yarn --cwd=./ui dev
```

## ⚡ Running Builds

```bash
yarn --cwd=./ui build
yarn --cwd=./ui preview
```

## ⚡ Generating Latest Schema

When the schema files change, the UI also is updated.
This is kicked off when the `generate-schema` make target runs.

```bash
# Generates:
# → ui/src/config.d.ts
# → ui/src/cel.d.ts
# → ui/public/config.d.ts
# → ui/public/cel.d.ts
# → ui/public/config-schema.json
# → ui/public/cel-schema.json
make generate-schema
```

## 🧭 Navigation Structure

**OLD Section** (Original Features)

- 🏠 Dashboard · 🔌 Listeners · 🛣️ Routes · 🔧 Backends · 📋 Policies · 🎮 Playground

**🤖 LLM Section**

- Overview · Models · Logs · Metrics · Playground

**🔗 MCP Section** (Model Context Protocol)

- Overview · Servers · Logs · Metrics · Playground

**🚦 Traffic Section**

- Overview · Routing · Logs · Metrics

**⚡ CEL Playground** (Standalone)

- CEL expression editor and testing

## Tech Stack

### Core

- **React 19** with TypeScript
- **Vite** for build tooling

### State Management

- **React Context** for global state (theme, server, loading, wizard)
- **SWR** for server data fetching and caching

### UI Components

- **Ant Design** components as base, customize with Emotion
- **Emotion CSS** for component styles
- **CSS custom variables** from `theme.css` for theming
- **Framer Motion** for animations
- **Lucide React** for icons
- **ChartJS** for charts (donut, bars)
- Utilities in `src/styles/emotion.ts` and `src/styles/global.css`

### Styling

- **Emotion CSS** for customizing antd styles
- **Custom CSS variables** for theme (colors, spacing)
- **CSS flex layout** for layouts

## 🧪 Testing

E2E testing is provided through Playwright. Tests cover two backend modes:

- **Standard mode** — agentgateway running with a local config file (admin endpoint on `:15000`).
- **xDS mode** — agentgateway configured to consume xDS (admin endpoint on `:15001`). Only `xdsMode.spec.ts` runs in this project.

Playwright manages the lifecycle of every required server (agentgateway binaries and, in dev mode, Vite servers). You don't need to start anything yourself, just run the script.

**Prerequisite:** the `agentgateway` binary must be on your `PATH` (e.g. `cargo build --release --features ui && export PATH=$PWD/target/release:$PATH` from the repo root).

### Running the tests

Two run modes, both single-command:

```bash
yarn test:e2e       # tests run against compiled binary (does not pick up UI changes without rebuild)
yarn test:e2e:dev   # tests run against Vite dev servers, which proxy to the binaries
```

- `test:e2e` — Playwright starts two agentgateway binaries (standard + xDS), points all tests directly at them. This is what CI runs.
- `test:e2e:dev` — sets `E2E_DEV_UI=true`. Playwright additionally starts two Vite dev servers:
  - `yarn dev` on `:5173` → proxies to the standard binary on `:15000`
  - `yarn dev:xds` on `:5174` → proxies to the xDS binary on `:15001`

Tests target the Vite servers instead of the binaries. UI changes are picked up instantly via the Vite dev server, so no binary rebuild is required. Note: the binaries themselves still need to be rebuilt if you change backend code.

If you already have `yarn dev` (or `yarn dev:xds`) running in another terminal, Playwright reuses the existing server (`reuseExistingServer: true`).

### Playwright test scripts
`test:e2e` or `test:e2e:dev` can be used for these commands.

Run all tests via command line.
```
yarn test:e2e           # headless
yarn testLe2e --headed  # headed with popup browser
```

Run all tests via interactive UI interface
```
yarn test:e2e test --ui
```

Run individual test by name
```
yarn test:e2e -g "Name of the test"
```

Show HTML test run report:
```
yarn playwright show-report
```

Run codegen (useful for writing e2e tests)
```
yarn playwright codegen
```

### Testing configuration

E2E tests are configured in `ui/playwright.config.ts`. Notable knobs:

- `STANDARD_BASE_URL` / `XDS_BASE_URL` env vars — override the default base URLs (rarely needed locally).
- `E2E_DEV_UI=true` — toggle dev-server mode (set by `test:e2e:dev`).
- The standard binary uses `tests/fixtures/e2e-config.yaml` locally, and `../e2e-test-config.yaml` in CI (a writable copy created by the workflow so the repo fixture stays pristine).