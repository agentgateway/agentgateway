import { Link } from "@tanstack/react-router";
import { Bot, Network, Server } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { configWarnings, ensureLlmFrontendDefaults } from "../config";
import { refreshBaseCostsAndConfigure } from "../costs";
import { useConfigDumpMode, useGatewayConfig, useUpdateConfig } from "../hooks";
import { PageHeader, StatusBanner } from "../components/Primitives";
import { trafficStats } from "../traffic";
import {
  ReadonlyModeBanner,
  TrafficDumpOverview,
} from "./traffic/TrafficConfigDumpPanel";

export function HomePage() {
  const mode = useConfigDumpMode();
  const dumpMode = mode.data?.mode === "dump";
  const config = useGatewayConfig({
    enabled: Boolean(mode.data && mode.data.mode !== "dump"),
  });
  const update = useUpdateConfig();
  const [locallyEnabled, setLocallyEnabled] = useState<Set<StartupSurface>>(
    () => new Set(),
  );
  const hasLlm = Boolean(config.data?.llm);
  const hasMcp = Boolean(config.data?.mcp);
  const hasTraffic = Boolean(config.data && "binds" in config.data);
  const models = config.data?.llm?.models ?? [];
  const virtualModels = config.data?.llm?.virtualModels ?? [];
  const mcpServers = config.data?.mcp?.targets ?? [];
  const warnings = config.data ? configWarnings(config.data) : [];
  const callableModels = models.length + virtualModels.length;
  const traffic = trafficStats(config.data);
  const [startupEvaluated, setStartupEvaluated] = useState(false);
  const [startupFlow, setStartupFlow] = useState(false);
  const [costRefreshError, setCostRefreshError] = useState<string | null>(null);
  const showStartup = Boolean(config.data && startupFlow);
  const selectedSurfaces =
    Number(hasLlm || locallyEnabled.has("llm")) +
    Number(hasMcp || locallyEnabled.has("mcp")) +
    Number(hasTraffic || locallyEnabled.has("apis"));

  useEffect(() => {
    if (!config.data || startupEvaluated) return;
    setStartupFlow(!hasLlm && !hasMcp && !hasTraffic);
    setStartupEvaluated(true);
  }, [config.data, hasLlm, hasMcp, hasTraffic, startupEvaluated]);

  async function enableSurface(surface: StartupSurface) {
    setCostRefreshError(null);
    try {
      await update.mutateAsync((next) => {
        if (surface === "llm") {
          next.llm = {
            port: 4000,
            models: [],
            providers: [],
            virtualModels: [],
          };
          ensureLlmFrontendDefaults(next);
        } else if (surface === "mcp") {
          next.mcp = { port: 3000, targets: [] };
        } else {
          next.binds = [];
        }
      });
      setLocallyEnabled((current) => new Set(current).add(surface));
      if (surface === "llm") {
        try {
          await refreshBaseCostsAndConfigure(update);
        } catch (err) {
          setCostRefreshError(
            err instanceof Error
              ? err.message
              : "Failed to refresh base cost catalog",
          );
        }
      }
    } catch {
      // useUpdateConfig exposes the save error through update.isError.
    }
  }

  if (mode.isLoading || (!dumpMode && config.isLoading)) {
    return (
      <div className="page-stack">
        <StatusBanner state="loading" title="Loading gateway configuration" />
      </div>
    );
  }

  if (dumpMode) {
    return (
      <div className="page-stack">
        <PageHeader title="Gateway Overview" />
        <ReadonlyModeBanner />
        <TrafficDumpOverview dump={mode.data?.dump} />
      </div>
    );
  }

  if (showStartup) {
    return (
      <div className="startup-shell" onClick={() => setStartupFlow(false)}>
        <section
          className="startup-panel"
          role="dialog"
          aria-modal="true"
          aria-labelledby="startup-title"
          onClick={(event) => event.stopPropagation()}
        >
          <div className="startup-copy">
            <h2 id="startup-title">Welcome to Agentgateway</h2>
            <p>
              Agentgateway is a gateway that can route, secure, and observe LLM,
              MCP, and traditional API traffic. Select one or more capabilities
              to enable, then continue.
            </p>
          </div>

          {config.isError ? (
            <StatusBanner state="bad" title="Configuration API unavailable">
              {config.error.message}
            </StatusBanner>
          ) : null}
          {update.isError ? (
            <StatusBanner state="bad" title="Save failed">
              {update.error.message}
            </StatusBanner>
          ) : null}
          {costRefreshError ? (
            <StatusBanner state="warn" title="Cost catalog refresh failed">
              {costRefreshError}
            </StatusBanner>
          ) : null}

          <div className="startup-chip-grid">
            <StartupChip
              label="LLM"
              description="Models, keys, policies, and chat testing."
              enabled={hasLlm || locallyEnabled.has("llm")}
              disabled={update.isPending}
              icon={<Bot size={24} />}
              onClick={() => void enableSurface("llm")}
            />
            <StartupChip
              label="MCP"
              description="Servers, tools, and MCP playground flows."
              enabled={hasMcp || locallyEnabled.has("mcp")}
              disabled={update.isPending}
              icon={<Server size={24} />}
              onClick={() => void enableSurface("mcp")}
            />
            <StartupChip
              label="APIs"
              description="HTTP and TCP listeners, routes, and policy controls."
              enabled={hasTraffic || locallyEnabled.has("apis")}
              disabled={update.isPending}
              icon={<Network size={24} />}
              onClick={() => void enableSurface("apis")}
            />
          </div>

          {selectedSurfaces > 0 ? (
            <div className="startup-actions">
              <span>{selectedSurfaces} of 3 enabled</span>
              <button
                className="button primary"
                type="button"
                onClick={() => setStartupFlow(false)}
              >
                Continue
              </button>
            </div>
          ) : (
            <div className="startup-actions">
              <button
                className="button"
                type="button"
                onClick={() => setStartupFlow(false)}
              >
                Skip setup
              </button>
            </div>
          )}
        </section>
      </div>
    );
  }

  return (
    <div className="page-stack">
      <PageHeader title="Gateway Overview" />

      {config.isLoading ? (
        <StatusBanner state="loading" title="Loading gateway configuration" />
      ) : config.isError ? (
        <StatusBanner state="bad" title="Configuration API unavailable">
          {config.error.message}
        </StatusBanner>
      ) : costRefreshError ? (
        <StatusBanner state="warn" title="Cost catalog refresh failed">
          {costRefreshError}
        </StatusBanner>
      ) : !hasLlm && !hasMcp && !hasTraffic ? (
        <StatusBanner state="warn" title="No gateway surfaces enabled yet">
          Enable the capabilities you want to operate from the setup path.
        </StatusBanner>
      ) : warnings.length ? (
        <StatusBanner
          state="warn"
          title={`${warnings.length} warning${warnings.length === 1 ? "" : "s"}`}
        >
          <ul className="banner-warning-list">
            {warnings.map((warning) => (
              <li key={warning}>{warning}</li>
            ))}
          </ul>
        </StatusBanner>
      ) : null}

      <section className="surface-overview-list" aria-label="Gateway surfaces">
        <SurfaceRow
          title="LLM"
          icon={<Bot size={18} />}
          enabled={hasLlm}
          disabled={update.isPending}
          onEnable={() => void enableSurface("llm")}
          setupNeeded={callableModels === 0}
          setupText="Add a model before LLM traffic can be served."
          setupTo="/llm/models"
          setupHash="add=model"
          setupLabel="Set up models"
          overview={[
            `${models.length} ${models.length === 1 ? "model" : "models"}`,
            `${virtualModels.length} virtual ${virtualModels.length === 1 ? "model" : "models"}`,
            `${config.data?.llm?.providers?.length ?? 0} shared ${config.data?.llm?.providers?.length === 1 ? "provider" : "providers"}`,
            `Port ${config.data?.llm?.port ?? 4000}`,
          ]}
          links={[
            { to: "/llm/playground", label: "Open playground" },
            { to: "/llm/analytics", label: "Analytics" },
          ]}
        />
        <SurfaceRow
          title="MCP"
          icon={<Server size={18} />}
          enabled={hasMcp}
          disabled={update.isPending}
          onEnable={() => void enableSurface("mcp")}
          setupNeeded={mcpServers.length === 0}
          setupText="Add an MCP target before tools are available."
          setupTo="/mcp/servers"
          setupLabel="Set up servers"
          overview={[
            `${mcpServers.length} configured ${mcpServers.length === 1 ? "server" : "servers"}`,
            `Port ${config.data?.mcp?.port ?? 3000}`,
          ]}
          links={[{ to: "/mcp/playground", label: "Open playground" }]}
        />
        <SurfaceRow
          title="Traffic"
          icon={<Network size={18} />}
          enabled={hasTraffic}
          disabled={update.isPending}
          onEnable={() => void enableSurface("apis")}
          setupNeeded={traffic.listeners === 0}
          setupText="Add a listener before HTTP or TCP traffic can be served."
          setupTo="/traffic/listeners"
          setupLabel="Set up listeners"
          overview={[
            `${traffic.binds} ${traffic.binds === 1 ? "bind" : "binds"}`,
            `${traffic.listeners} ${traffic.listeners === 1 ? "listener" : "listeners"}`,
            `${traffic.httpRoutes + traffic.tcpRoutes} ${traffic.httpRoutes + traffic.tcpRoutes === 1 ? "route" : "routes"}`,
          ]}
          links={[{ to: "/traffic/routes", label: "Manage routes" }]}
        />
      </section>
    </div>
  );
}

type StartupSurface = "llm" | "mcp" | "apis";

function StartupChip(props: {
  description: string;
  disabled: boolean;
  enabled: boolean;
  icon: ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      className={props.enabled ? "startup-chip enabled" : "startup-chip"}
      type="button"
      disabled={props.disabled || props.enabled}
      onClick={props.onClick}
    >
      {props.icon}
      <strong>
        {props.enabled ? `${props.label} enabled` : `Enable ${props.label}`}
      </strong>
      <span>{props.description}</span>
    </button>
  );
}

function SurfaceRow(props: {
  disabled: boolean;
  enabled: boolean;
  icon: ReactNode;
  links: Array<{ label: string; to: string }>;
  onEnable: () => void;
  overview: string[];
  setupLabel: string;
  setupNeeded: boolean;
  setupText: string;
  setupHash?: string;
  setupTo: string;
  title: string;
}) {
  if (!props.enabled) {
    return (
      <div className="surface-row compact">
        <div className="surface-row-title">
          {props.icon}
          <strong>{props.title}</strong>
          <span>Not enabled</span>
        </div>
        <button
          className="button"
          type="button"
          disabled={props.disabled}
          onClick={props.onEnable}
        >
          Enable {props.title}
        </button>
      </div>
    );
  }

  return (
    <div
      className={props.setupNeeded ? "surface-row needs-setup" : "surface-row"}
    >
      <div className="surface-row-main">
        <div className="surface-row-title">
          {props.icon}
          <strong>{props.title}</strong>
          <span>Enabled</span>
        </div>
        {props.setupNeeded ? (
          <p>{props.setupText}</p>
        ) : (
          <div className="surface-metrics">
            {props.overview.map((item) => (
              <span key={item}>{item}</span>
            ))}
          </div>
        )}
      </div>
      <div className="surface-row-actions">
        {props.setupNeeded ? (
          <Link
            className="button primary"
            to={props.setupTo}
            hash={props.setupHash}
          >
            {props.setupLabel}
          </Link>
        ) : (
          props.links.map((link) => (
            <Link key={link.to} className="button" to={link.to}>
              {link.label}
            </Link>
          ))
        )}
      </div>
    </div>
  );
}
