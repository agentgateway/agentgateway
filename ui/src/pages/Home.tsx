import { tr, translateText } from "../i18n";
import { Link } from "@tanstack/react-router";
import { Bot, Network, Server, Settings } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import {
  configWarnings,
  ensureLlm,
  ensureLlmFrontendDefaults,
  ensureMcp,
  startupLlmConfig,
  startupMcpConfig,
} from "../config";
import { refreshBaseCostsAndConfigure } from "../costs";
import { useConfigDumpMode, useGatewayConfig, useUpdateConfig } from "../hooks";
import { PageHeader, StatusBanner } from "../components/Primitives";
import { trafficStats } from "../traffic";
import {
  ReadonlyModeBanner,
  TrafficDumpOverview,
} from "./traffic/TrafficConfigDumpPanel";
import { LlmSettingsDrawer } from "./models/LlmSettingsDrawer";
import { useSchemaHelp } from "../schemaHelp";
import { McpSettingsDrawer } from "./McpServers";
import type { GatewayConfig } from "../types";

const uiAuthPolicyKeys = [
  "oidc",
  "jwtAuth",
  "extAuthz",
  "basicAuth",
  "apiKey",
  "authorization",
];

export function HomePage() {
  const mode = useConfigDumpMode();
  const dumpMode = mode.data?.mode === "dump";
  const config = useGatewayConfig({
    enabled: Boolean(mode.data && mode.data.mode !== "dump"),
  });
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const [locallyEnabled, setLocallyEnabled] = useState<Set<StartupSurface>>(
    () => new Set(),
  );
  const hasLlm = Boolean(config.data?.llm);
  const hasMcp = Boolean(config.data?.mcp);
  const hasTraffic = Boolean(
    config.data &&
    (Boolean(config.data.binds?.length) ||
      "gateways" in config.data ||
      "routes" in config.data ||
      "tcpRoutes" in config.data),
  );
  const hasBinds = Boolean(config.data?.binds?.length);
  const models = config.data?.llm?.models ?? [];
  const virtualModels = config.data?.llm?.virtualModels ?? [];
  const mcpServers = config.data?.mcp?.targets ?? [];
  const warnings = config.data ? configWarnings(config.data) : [];
  const uiGatewayNeedsAuthWarning = uiExposedWithoutAuth(config.data);
  const callableModels = models.length + virtualModels.length;
  const traffic = trafficStats(config.data);
  const [startupEvaluated, setStartupEvaluated] = useState(false);
  const [startupFlow, setStartupFlow] = useState(false);
  const [costRefreshError, setCostRefreshError] = useState<string | null>(null);
  const [llmSettingsOpen, setLlmSettingsOpen] = useState(false);
  const [mcpSettingsOpen, setMcpSettingsOpen] = useState(false);
  const showStartup = Boolean(config.data && startupFlow);
  const selectedSurfaces =
    Number(hasLlm || locallyEnabled.has("llm")) +
    Number(hasMcp || locallyEnabled.has("mcp")) +
    Number(hasTraffic || locallyEnabled.has("apis"));

  useEffect(() => {
    if (!config.data || startupEvaluated) return;
    setStartupFlow(
      !hasLlm &&
        !hasMcp &&
        (!hasTraffic || isDefaultUiGatewayScaffold(config.data)),
    );
    setStartupEvaluated(true);
  }, [config.data, hasLlm, hasMcp, hasTraffic, startupEvaluated]);

  async function enableSurface(surface: StartupSurface) {
    setCostRefreshError(null);
    try {
      await update.mutateAsync((next) => {
        if (surface === "llm") {
          next.llm = startupLlmConfig(next, 4000);
          ensureLlmFrontendDefaults(next);
        } else if (surface === "mcp") {
          next.mcp = startupMcpConfig(next, 3000);
        } else {
          next.gateways ??= { default: { port: 8080 } };
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
        <StatusBanner
          state="loading"
          title={tr("copy.loadingGatewayConfiguration")}
        />
      </div>
    );
  }

  if (dumpMode) {
    return (
      <div className="page-stack">
        <PageHeader title={tr("copy.gatewayOverview")} />
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
            <h2 id="startup-title">{tr("copy.welcomeToAgentgateway")}</h2>
            <p>
              {tr(
                "copy.agentgatewayIsAGatewayThatCanRouteSecureAndObserveLlmMcpAndTraditionalApiTraffic_sbsjep",
              )}
            </p>
          </div>

          {config.isError ? (
            <StatusBanner
              state="bad"
              title={tr("copy.configurationApiUnavailable")}
            >
              {config.error.message}
            </StatusBanner>
          ) : null}
          {update.isError ? (
            <StatusBanner state="bad" title={tr("copy.saveFailed")}>
              {update.error.message}
            </StatusBanner>
          ) : null}
          {costRefreshError ? (
            <StatusBanner
              state="warn"
              title={tr("copy.costCatalogRefreshFailed")}
            >
              {costRefreshError}
            </StatusBanner>
          ) : null}

          <div className="startup-chip-grid">
            <StartupChip
              label="LLM"
              description={tr("copy.modelsKeysPoliciesAndChatTesting")}
              enabled={hasLlm || locallyEnabled.has("llm")}
              disabled={update.isPending}
              icon={<Bot size={24} />}
              onClick={() => void enableSurface("llm")}
            />
            <StartupChip
              label="MCP"
              description={tr("copy.serversToolsAndMcpPlaygroundFlows")}
              enabled={hasMcp || locallyEnabled.has("mcp")}
              disabled={update.isPending}
              icon={<Server size={24} />}
              onClick={() => void enableSurface("mcp")}
            />
            <StartupChip
              label={tr("copy.apis")}
              description={tr(
                "copy.httpAndTcpListenersRoutesAndPolicyControls",
              )}
              enabled={hasTraffic || locallyEnabled.has("apis")}
              disabled={update.isPending}
              icon={<Network size={24} />}
              onClick={() => void enableSurface("apis")}
            />
          </div>

          {selectedSurfaces > 0 ? (
            <div className="startup-actions">
              <span>{tr("copy.valueOf3Enabled", [selectedSurfaces])}</span>
              <button
                className="button primary"
                type="button"
                onClick={() => setStartupFlow(false)}
              >
                {tr("copy.continue")}
              </button>
            </div>
          ) : (
            <div className="startup-actions">
              <button
                className="button"
                type="button"
                onClick={() => setStartupFlow(false)}
              >
                {tr("copy.skipSetup")}
              </button>
            </div>
          )}
        </section>
      </div>
    );
  }

  return (
    <div className="page-stack">
      <PageHeader title={tr("copy.gatewayOverview")} />

      {config.isLoading ? (
        <StatusBanner
          state="loading"
          title={tr("copy.loadingGatewayConfiguration")}
        />
      ) : config.isError ? (
        <StatusBanner
          state="bad"
          title={tr("copy.configurationApiUnavailable")}
        >
          {config.error.message}
        </StatusBanner>
      ) : costRefreshError ? (
        <StatusBanner state="warn" title={tr("copy.costCatalogRefreshFailed")}>
          {costRefreshError}
        </StatusBanner>
      ) : !hasLlm && !hasMcp && !hasTraffic ? (
        <StatusBanner
          state="warn"
          title={tr("copy.noGatewaySurfacesEnabledYet")}
        >
          {tr("copy.enableTheCapabilitiesYouWantToOperateFromTheSetupPath")}
        </StatusBanner>
      ) : warnings.length ? (
        <StatusBanner
          state="warn"
          title={tr("copy.valueWarningValue", { count: warnings.length })}
        >
          <ul className="banner-warning-list">
            {warnings.map((warning) => (
              <li key={warning}>{configWarningLabel(warning)}</li>
            ))}
          </ul>
        </StatusBanner>
      ) : null}
      {uiGatewayNeedsAuthWarning ? (
        <StatusBanner
          state="warn"
          title={tr("copy.uiIsExposedWithoutAuthentication")}
          action={
            <Link className="button" to="/settings">
              {tr("copy.configureUiPolicies")}
            </Link>
          }
        >
          {tr(
            "copy.unauthenticatedUsersCanAccessTheUiConsiderAddingAuthenticationOrAuthorizationPol_qnhsta",
          )}
        </StatusBanner>
      ) : null}

      <section
        className="surface-overview-list"
        aria-label={tr("copy.gatewaySurfaces")}
      >
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
            tr("copy.valueModels", { count: models.length }),
            tr("copy.valueVirtualModels", { count: virtualModels.length }),
            tr("copy.valueSharedProviders", {
              count: config.data?.llm?.providers?.length ?? 0,
            }),
            surfaceEndpointLabel(
              config.data?.llm?.gateways,
              config.data?.llm?.port ?? 4000,
            ),
          ]}
          actions={
            <button
              className="button"
              type="button"
              disabled={update.isPending}
              onClick={() => setLlmSettingsOpen(true)}
            >
              <Settings size={16} />
              {tr("copy.settings")}
            </button>
          }
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
            tr("copy.valueConfiguredServers", { count: mcpServers.length }),
            surfaceEndpointLabel(
              config.data?.mcp?.gateways,
              config.data?.mcp?.port ?? 3000,
            ),
          ]}
          actions={
            <button
              className="button"
              type="button"
              disabled={update.isPending}
              onClick={() => setMcpSettingsOpen(true)}
            >
              <Settings size={16} />
              {tr("copy.settings")}
            </button>
          }
        />
        <SurfaceRow
          title={tr("copy.traffic")}
          icon={<Network size={18} />}
          enabled={hasTraffic}
          disabled={update.isPending}
          onEnable={() => void enableSurface("apis")}
          setupNeeded={
            hasBinds ? traffic.listeners === 0 : traffic.gateways === 0
          }
          setupText={
            hasBinds
              ? "Add a listener before HTTP or TCP traffic can be served."
              : "Add a gateway before HTTP traffic can be served."
          }
          setupTo={hasBinds ? "/traffic/listeners" : "/traffic/gateways"}
          setupLabel={hasBinds ? "Set up listeners" : "Set up gateways"}
          overview={
            hasBinds
              ? [
                  tr("copy.valueBinds", { count: traffic.binds }),
                  tr("copy.valueListeners", { count: traffic.listeners }),
                  tr("copy.valueRoutes", {
                    count: traffic.httpRoutes + traffic.tcpRoutes,
                  }),
                ]
              : [
                  tr("copy.valueGateways", { count: traffic.gateways }),
                  tr("copy.valueRoutes", { count: traffic.httpRoutes }),
                ]
          }
        />
      </section>
      {llmSettingsOpen ? (
        <LlmSettingsDrawer
          config={config.data}
          llm={config.data?.llm}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onClose={() => setLlmSettingsOpen(false)}
          onSave={(settings) =>
            update.mutate(
              (next) => {
                Object.assign(ensureLlm(next), settings);
              },
              {
                onSuccess: () => setLlmSettingsOpen(false),
              },
            )
          }
        />
      ) : null}
      {mcpSettingsOpen ? (
        <McpSettingsDrawer
          config={config.data}
          mcp={config.data?.mcp}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onClose={() => setMcpSettingsOpen(false)}
          onSave={(settings) =>
            update.mutate(
              (next) => {
                Object.assign(ensureMcp(next), settings);
              },
              {
                onSuccess: () => setMcpSettingsOpen(false),
              },
            )
          }
        />
      ) : null}
    </div>
  );
}

function surfaceEndpointLabel(
  gateways: string | string[] | undefined,
  port: number,
) {
  if (!gateways) return tr("copy.portValue", port);
  return tr(
    "copy.gatewayValue",
    Array.isArray(gateways) ? gateways.join(", ") : gateways,
  );
}

function configWarningLabel(warning: string) {
  const virtualKeyMode = warning.match(
    /^Virtual API key mode is ([^;]+); unauthenticated requests may be accepted\.$/,
  );
  if (virtualKeyMode) {
    return tr(
      "copy.virtualApiKeyModeIsValueUnauthenticatedRequestsMayBeAccepted",
      translateText(virtualKeyMode[1]),
    );
  }
  const modelWarning = warning.match(/^([^:]+): (.+)$/);
  if (modelWarning)
    return `${modelWarning[1]}：${translateText(modelWarning[2])}`;
  return translateText(warning);
}

function uiExposedWithoutAuth(config: GatewayConfig | null | undefined) {
  if (!uiGateway(config)) return false;
  const policies = config?.ui?.policies as Record<string, unknown> | undefined;
  return !uiAuthPolicyKeys.some((key) => Boolean(policies?.[key]));
}

function uiGateway(config: GatewayConfig | null | undefined) {
  const gateways = config?.ui?.gateways;
  if (Array.isArray(gateways)) return gateways[0];
  if (gateways) return gateways;
  return config?.ui && config.gateways?.default ? "default" : undefined;
}

function isDefaultUiGatewayScaffold(config: GatewayConfig) {
  if (!config.ui || uiGateway(config) !== "default") return false;
  if (
    config.binds?.length ||
    config.routes?.length ||
    config.tcpRoutes?.length
  ) {
    return false;
  }
  const gatewayNames = Object.keys(config.gateways ?? {});
  return gatewayNames.length === 1 && gatewayNames[0] === "default";
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
        {props.enabled
          ? tr("copy.valueEnabled", [props.label])
          : tr("copy.enableValue", [props.label])}
      </strong>
      <span>{props.description}</span>
    </button>
  );
}

function SurfaceRow(props: {
  disabled: boolean;
  enabled: boolean;
  icon: ReactNode;
  actions?: ReactNode;
  links?: Array<{ label: string; to: string }>;
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
          <span>{tr("copy.notEnabled")}</span>
        </div>
        <button
          className="button"
          type="button"
          disabled={props.disabled}
          onClick={props.onEnable}
        >
          {tr("copy.enableValue", [props.title])}
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
          <span>{tr("copy.enabled_17fi4vy")}</span>
        </div>
        {props.setupNeeded ? (
          <p>{translateText(props.setupText)}</p>
        ) : (
          <div className="surface-metrics">
            {props.overview.map((item) => (
              <span key={item}>{item}</span>
            ))}
          </div>
        )}
      </div>
      <div className="surface-row-actions">
        {!props.setupNeeded
          ? (props.actions ??
            props.links?.map((link) => (
              <Link key={link.to} className="button" to={link.to}>
                {translateText(link.label)}
              </Link>
            )))
          : null}
        <Link
          className="button primary"
          to={props.setupTo}
          hash={props.setupHash}
        >
          {translateText(props.setupLabel)}
        </Link>
      </div>
    </div>
  );
}
