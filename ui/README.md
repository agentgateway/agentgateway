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

First, make sure agentgateway is running as well. The UI must run on port 3000, so any config files that have a port bind to 3000 must be updated (you can change them to 3001 before starting the program):

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
