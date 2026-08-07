import { Eye, Server } from "lucide-react";
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

type McpBackendEntry = {
  backend?: {
    mcp?: {
      name?: string;
      namespace?: string;
      [k: string]: unknown;
    };
    [k: string]: unknown;
  };
  [key: string]: unknown;
};

function isMcpBackend(entry: unknown): entry is McpBackendEntry {
  return Boolean(
    entry &&
      typeof entry === "object" &&
      (entry as McpBackendEntry).backend &&
      typeof (entry as McpBackendEntry).backend === "object" &&
      (entry as McpBackendEntry).backend?.mcp,
  );
}

function backendKey(entry: McpBackendEntry, index: number) {
  const mcp = entry.backend?.mcp;
  return mcp
    ? `${mcp.namespace ?? "default"}/${mcp.name ?? index}`
    : `mcp-backend-${index}`;
}

export function McpBackendsDumpPage() {
  const mode = useConfigDumpMode();
  const [selectedKey, setSelectedKey] = useStickyQueryParam("mcp-backend");
  const dumpMode = mode.data?.mode === "dump";
  const backends = ((mode.data?.dump?.backends ?? []) as unknown[]).filter(
    isMcpBackend,
  );
  const selectedIndex = backends.findIndex(
    (entry, index) => backendKey(entry, index) === selectedKey,
  );
  const selected = selectedIndex >= 0 ? backends[selectedIndex] : undefined;

  return (
    <div className="page-stack">
      <PageHeader
        title="MCP Servers"
        description="Read-only MCP backends from the active gateway dump."
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
            title="No MCP backends on this gateway"
            description="This gateway currently routes zero MCP backends — no AgentgatewayBackend of kind mcp targets it. If MCP traffic is ever routed through this gateway, real MCP backends will appear here automatically."
          />
        ) : (
          <div className="table-wrap">
            <table className="dump-policies-table">
              <thead>
                <tr>
                  <th>Server</th>
                  <th aria-label="Actions" />
                </tr>
              </thead>
              <tbody>
                {backends.map((entry, index) => {
                  const mcp = entry.backend?.mcp;
                  const key = backendKey(entry, index);
                  return (
                    <tr key={key}>
                      <td>
                        <div className="resource-name-cell">
                          <strong>{mcp?.name ?? "server"}</strong>
                          <small>{mcp?.namespace ?? "default"}</small>
                        </div>
                      </td>
                      <td className="row-actions">
                        <Tooltip content="View backend">
                          <button
                            className="icon-button"
                            type="button"
                            aria-label={`View ${mcp?.name ?? "backend"}`}
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
          title={selected.backend?.mcp?.name ?? "MCP backend"}
          headerActions={
            <span className="badge">
              <Server size={14} /> MCP backend
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
