import { Eye, Bot } from "lucide-react";
import {
  Drawer,
  EmptyState,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import { useStickyQueryParam } from "../drawerRouteState";
import { useConfigDumpMode } from "../hooks";
import { ReadonlyModeBanner } from "./traffic/TrafficConfigDumpPanel";

type AiBackendEndpoint = {
  name?: string;
  provider?: Record<string, unknown> | null;
  hostOverride?: string | null;
  pathOverride?: string | null;
} | null;

type AiBackendInfo = {
  health?: number;
  requestLatency?: number;
  pendingRequests?: number;
  totalRequests?: number;
  consecutiveFailures?: number;
  timesEjected?: number;
} | null;

type AiBackendActiveEntry = {
  endpoint?: AiBackendEndpoint;
  info?: AiBackendInfo;
  capacity?: number;
};

type AiBackendProviderEntry = {
  // Keyed by the endpoint's own name (arbitrary per-config), not a fixed
  // field name — e.g. {"backend": {...}} or {"nvidia-nim-x": {...}}.
  active?: Record<string, AiBackendActiveEntry> | null;
  rejected?: unknown;
};

type AiBackend = {
  name?: string;
  namespace?: string;
  target?: {
    providers?: AiBackendProviderEntry[];
  } | null;
};

type BackendEntry = {
  backend?: {
    ai?: AiBackend;
    [k: string]: unknown;
  };
  [key: string]: unknown;
};

function backendKey(entry: BackendEntry, index: number) {
  const ai = entry.backend?.ai;
  return ai ? `${ai.namespace ?? "default"}/${ai.name ?? index}` : `backend-${index}`;
}

function isAiBackend(entry: unknown): entry is BackendEntry {
  return Boolean(
    entry &&
      typeof entry === "object" &&
      (entry as BackendEntry).backend &&
      typeof (entry as BackendEntry).backend === "object" &&
      (entry as BackendEntry).backend?.ai,
  );
}

function firstActiveProvider(ai: AiBackend | undefined) {
  const providers = ai?.target?.providers ?? [];
  return providers.find((p) => p.active) ?? providers[0];
}

function firstActiveEntry(
  provider: AiBackendProviderEntry | undefined,
): AiBackendActiveEntry | null {
  if (!provider?.active) return null;
  const values = Object.values(provider.active);
  return values[0] ?? null;
}

function providerTypeLabel(endpoint: AiBackendEndpoint) {
  const provider = endpoint?.provider;
  if (!provider || typeof provider !== "object") return "unknown";
  const key = Object.keys(provider)[0];
  return key ?? "unknown";
}

function healthLabel(info: AiBackendInfo) {
  if (!info || typeof info.health !== "number") return "unknown";
  if (info.health >= 1) return "healthy";
  if (info.health > 0) return "degraded";
  return "unhealthy";
}

export function LlmBackendsDumpPage() {
  const mode = useConfigDumpMode();
  const [selectedKey, setSelectedKey] = useStickyQueryParam("backend");
  const dumpMode = mode.data?.mode === "dump";
  const backends = ((mode.data?.dump?.backends ?? []) as unknown[]).filter(
    isAiBackend,
  );
  const selectedIndex = backends.findIndex(
    (entry, index) => backendKey(entry, index) === selectedKey,
  );
  const selected = selectedIndex >= 0 ? backends[selectedIndex] : undefined;

  return (
    <div className="page-stack">
      <PageHeader
        title="Models"
        description="Read-only LLM backends (models and providers) from the active gateway dump."
      />
      <ReadonlyModeBanner />

      <Panel>
        {mode.isLoading ? (
          <StatusBanner state="loading" title="Loading runtime backends" />
        ) : mode.error ? (
          <StatusBanner state="bad" title="Config dump unavailable">
            {mode.error.message}
          </StatusBanner>
        ) : !dumpMode ? (
          <StatusBanner state="warn" title="Readonly backends unavailable">
            The runtime backend list is only available when the gateway is
            running from XDS config.
          </StatusBanner>
        ) : !backends.length ? (
          <EmptyState
            title="No LLM backends"
            description="No AI/LLM backends are present in the active gateway dump."
          />
        ) : (
          <div className="table-wrap">
            <table className="dump-policies-table">
              <thead>
                <tr>
                  <th>Model</th>
                  <th>Provider</th>
                  <th>Endpoint</th>
                  <th>Health</th>
                  <th>Total requests</th>
                  <th aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {backends.map((entry, index) => {
                  const ai = entry.backend?.ai;
                  const key = backendKey(entry, index);
                  const provider = firstActiveProvider(ai);
                  const activeEntry = firstActiveEntry(provider);
                  const endpoint = activeEntry?.endpoint ?? null;
                  const info = activeEntry?.info ?? null;
                  return (
                    <tr key={key}>
                      <td>
                        <div className="resource-name-cell">
                          <strong>{ai?.name ?? "model"}</strong>
                          <small>{ai?.namespace ?? "default"}</small>
                        </div>
                      </td>
                      <td>
                        <span className="badge">
                          {providerTypeLabel(endpoint)}
                        </span>
                      </td>
                      <td>{endpoint?.hostOverride ?? "unknown"}</td>
                      <td>
                        <span className={`badge status-${healthLabel(info)}`}>
                          {healthLabel(info)}
                        </span>
                      </td>
                      <td>{info?.totalRequests ?? 0}</td>
                      <td className="row-actions">
                        <Tooltip content="View backend">
                          <button
                            className="icon-button"
                            type="button"
                            aria-label={`View ${ai?.name ?? "backend"}`}
                            onClick={() => setSelectedKey(key)}
                          >
                            <Eye size={16} />
                          </button>
                        </Tooltip>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </Panel>

      {selected ? (
        <Drawer
          title={selected.backend?.ai?.name ?? "Backend"}
          headerActions={
            <span className="badge">
              <Bot size={14} /> LLM backend
            </span>
          }
          onClose={() => setSelectedKey(null)}
        >
          <YamlBlock value={selected} />
        </Drawer>
      ) : null}
    </div>
  );
}
