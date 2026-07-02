// Thin wrappers that gate the original (standalone-mode-only) LLM pages
// behind an XDS/dump-mode check, swapping in an honest "not available"
// placeholder instead. Implemented as parent-level component swaps (not
// conditional hook calls inside the original components) so there is no
// Rules-of-Hooks risk: React fully mounts/unmounts one tree or the other.
import { StatusBanner } from "../components/Primitives";
import { useConfigDumpMode } from "../hooks";
import { AnalyticsPage, LogsPage } from "./Logs";
import { ClientSetupPage } from "./ClientSetup";
import { CostsPage } from "./Costs";
import { PlaygroundPage } from "./Playground";
import { McpPlaygroundPage } from "./McpPlayground";
import { XdsUnavailablePage } from "./XdsUnavailable";

// NOTE: all guards below use a parent-level full-component swap (never
// mounting the original page in dump mode) rather than an early-return
// inside the original component after its hooks run. CostsPage originally
// used the latter pattern and caused an infinite render loop in dump mode:
// configuredCostSources(undefined) returns a fresh `[]` reference every
// render, so a useEffect keyed on that array kept "changing" and
// re-triggering forever. Full component swap avoids this class of bug
// entirely since the original component's hooks never run at all.
export function GuardedCostsPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return <StatusBanner state="loading" title="Loading" />;
  }
  if (mode.data?.mode === "dump") {
    return (
      <XdsUnavailablePage
        title="Costs"
        description="Per-model/provider cost tracking."
        reason="Cost tracking here is a local, standalone-mode-only bookkeeping feature — it reads a config flag and a local log store that don't exist when the gateway is Kubernetes/XDS-managed. If this deployment exports OpenTelemetry traces/metrics, that telemetry stack is a stronger source of per-request cost/latency/token data than this page would show."
      />
    );
  }
  return <CostsPage />;
}

export function GuardedLogsPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return <StatusBanner state="loading" title="Loading" />;
  }
  if (mode.data?.mode === "dump") {
    return (
      <XdsUnavailablePage
        title="Logs"
        description="Request/response log viewer."
        reason="This page reads from a local prompt/response log store that only exists in standalone mode — there is no equivalent surfaced by the XDS config dump. Request logs for an XDS-managed gateway are typically available via kubectl logs and/or whatever observability stack (OpenTelemetry, etc.) this deployment exports to."
      />
    );
  }
  return <LogsPage />;
}

export function GuardedAnalyticsPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return <StatusBanner state="loading" title="Loading" />;
  }
  if (mode.data?.mode === "dump") {
    return (
      <XdsUnavailablePage
        title="Analytics"
        description="Usage charts."
        reason="Analytics here reads the same local, standalone-mode-only log store as the Logs page, aggregated in the browser. For an XDS-managed gateway, use whatever telemetry stack this deployment exports OpenTelemetry data to instead."
      />
    );
  }
  return <AnalyticsPage />;
}

export function GuardedClientSetupPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return <StatusBanner state="loading" title="Loading" />;
  }
  if (mode.data?.mode === "dump") {
    return (
      <XdsUnavailablePage
        title="Client Setup"
        description="Generated client/SDK snippets for calling this gateway."
        reason="This wizard builds snippets from the local writable model/virtual-key config, which is empty in XDS mode. See the Models page for the real list of configured models, and use this gateway's real hostname as the base URL — a plain curl/OpenAI-SDK call against /v1/chat/completions works the same way this wizard would have generated."
      />
    );
  }
  return <ClientSetupPage />;
}

export function GuardedPlaygroundPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return <StatusBanner state="loading" title="Loading" />;
  }
  if (mode.data?.mode === "dump") {
    return (
      <XdsUnavailablePage
        title="Chat Playground"
        description="Send a real chat completion request through the configured gateway."
        reason="This form makes a direct browser fetch() to the gateway's own /v1/chat/completions, which requires both a CORS-allow policy for this origin and a writable local model list — neither works in XDS mode (the 'Apply CORS' write is a no-op, and the model dropdown only reads the local config store, not the XDS dump). The exact same request is one curl call away: curl -X POST &lt;gateway-url&gt;/v1/chat/completions -H 'Content-Type: application/json' -d '{&quot;model&quot;:&quot;&lt;model-name-from-Models-page&gt;&quot;,&quot;messages&quot;:[{&quot;role&quot;:&quot;user&quot;,&quot;content&quot;:&quot;hello&quot;}]}' — no CORS restriction applies outside a browser."
      />
    );
  }
  return <PlaygroundPage />;
}

export function GuardedMcpPlaygroundPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return <StatusBanner state="loading" title="Loading" />;
  }
  if (mode.data?.mode === "dump") {
    return (
      <XdsUnavailablePage
        title="Tool Playground"
        description="Send a real MCP JSON-RPC call through the configured gateway."
        reason="Same limitation as Chat Playground: a direct browser fetch() blocked by CORS in XDS mode, plus this gateway currently has no MCP backends configured at all (see MCP > Servers). Use a direct JSON-RPC call from curl/an MCP client instead."
      />
    );
  }
  return <McpPlaygroundPage />;
}
