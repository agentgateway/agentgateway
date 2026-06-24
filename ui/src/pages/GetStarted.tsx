import { Link, useNavigate } from "@tanstack/react-router";
import { Bot, Network, Server } from "lucide-react";
import { useEffect, useState } from "react";
import { ensureLlmFrontendDefaults } from "../config";
import { refreshBaseCostsAndConfigure } from "../costs";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import {
  Field,
  PageHeader,
  Panel,
  StatusBanner,
} from "../components/Primitives";
import type { GatewayConfig } from "../types";

type SurfaceKind = "llm" | "mcp" | "traffic";

const surfaceConfig: Record<
  SurfaceKind,
  {
    title: string;
    description: string;
    icon: typeof Bot;
    enabled: (config: GatewayConfig | undefined) => boolean;
    destination: string;
    destinationLabel: string;
  }
> = {
  llm: {
    title: "Enable LLM",
    description:
      "Create the LLM configuration section so models, providers, keys, guardrails, logs, and playground tools can be configured.",
    icon: Bot,
    enabled: (config) => Boolean(config?.llm),
    destination: "/llm/models",
    destinationLabel: "Continue to models",
  },
  mcp: {
    title: "Enable MCP",
    description:
      "Create the MCP configuration section so servers and MCP playground tools can be configured.",
    icon: Server,
    enabled: (config) => Boolean(config?.mcp),
    destination: "/mcp/servers",
    destinationLabel: "Continue to servers",
  },
  traffic: {
    title: "Enable Traffic",
    description:
      "Create the traffic configuration section so HTTP and TCP listeners, routes, backends, and policies can be configured.",
    icon: Network,
    enabled: (config) => Boolean(config && "binds" in config),
    destination: "/traffic/listeners",
    destinationLabel: "Continue to listeners",
  },
};

export function LlmGetStartedPage() {
  return <GetStartedPage surface="llm" />;
}

export function McpGetStartedPage() {
  return <GetStartedPage surface="mcp" />;
}

export function TrafficGetStartedPage() {
  return <GetStartedPage surface="traffic" />;
}

function GetStartedPage(props: { surface: SurfaceKind }) {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const navigate = useNavigate();
  const surface = surfaceConfig[props.surface];
  const Icon = surface.icon;
  const enabled = surface.enabled(config.data);
  const [port, setPort] = useState(() =>
    String(defaultSurfacePort(props.surface)),
  );

  useEffect(() => {
    if (!config.isLoading && !config.isError && enabled) {
      void navigate({ to: surface.destination, replace: true });
    }
  }, [
    config.isError,
    config.isLoading,
    enabled,
    navigate,
    surface.destination,
  ]);

  async function enable() {
    if (enabled) {
      void navigate({ to: surface.destination });
      return;
    }
    try {
      await update.mutateAsync((next) => {
        if (props.surface === "llm") {
          next.llm = next.llm ?? {
            port: parsePort(port, 4000),
            models: [],
            providers: [],
            virtualModels: [],
          };
          ensureLlmFrontendDefaults(next);
        } else if (props.surface === "mcp") {
          next.mcp = next.mcp ?? {
            port: parsePort(port, defaultSurfacePort(props.surface)),
            targets: [],
          };
        } else if (!("binds" in next)) {
          next.binds = [];
        }
      });
      void navigate({ to: surface.destination });
      if (props.surface === "llm") {
        void refreshBaseCostsAndConfigure(update).catch(() => undefined);
      }
    } catch {
      // useUpdateConfig exposes the save error through update.isError.
    }
  }

  if (!config.isLoading && !config.isError && enabled) {
    return (
      <div className="page-stack">
        <StatusBanner
          state="loading"
          title={`Opening ${surface.destinationLabel.toLowerCase()}`}
        />
      </div>
    );
  }

  return (
    <div className="page-stack">
      <PageHeader title={surface.title} description={surface.description} />

      {config.isLoading ? (
        <StatusBanner state="loading" title="Loading gateway configuration" />
      ) : null}
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

      <Panel className="surface-enable-panel">
        <div className="surface-enable-heading">
          <span className="policy-form-section-icon">
            <Icon size={18} />
          </span>
          <div>
            <h3>
              {enabled
                ? `${surface.title.replace("Enable ", "")} is enabled`
                : surface.title}
            </h3>
            <p>
              {enabled
                ? "The top-level configuration section already exists."
                : surface.description}
            </p>
          </div>
        </div>

        {!enabled && (props.surface === "llm" || props.surface === "mcp") ? (
          <details className="schema-details">
            <summary>Advanced</summary>
            <Field label="Port">
              <input
                value={port}
                inputMode="numeric"
                onChange={(event) => setPort(event.target.value)}
                placeholder={String(defaultSurfacePort(props.surface))}
              />
            </Field>
          </details>
        ) : null}

        <div className="button-row">
          {enabled ? (
            <Link className="button primary" to={surface.destination}>
              {surface.destinationLabel}
            </Link>
          ) : (
            <button
              className="button primary"
              type="button"
              disabled={config.isLoading || update.isPending}
              onClick={() => void enable()}
            >
              Enable
            </button>
          )}
          <Link className="button" to="/">
            Back to home
          </Link>
        </div>
      </Panel>
    </div>
  );
}

function parsePort(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function defaultSurfacePort(surface: SurfaceKind) {
  if (surface === "llm") return 4000;
  return 3000;
}
