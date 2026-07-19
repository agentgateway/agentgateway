import { tr } from "../i18n";
import { Pencil, Plus, Server, SlidersHorizontal, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import {
  ensureMcp,
  makeEmptyMcpTarget,
  removeMcpTarget,
  upsertMcpTarget,
} from "../config";
import { EnumSelector } from "../components/EnumSelector";
import {
  GatewayBindingEditor,
  type GatewayBindingValue,
} from "../components/GatewayBindingEditor";
import { ConfigDiffSaveActions } from "../components/ConfigDiffDrawer";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { useStickyQueryParam } from "../drawerRouteState";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import {
  ConfirmDialog,
  Drawer,
  EmptyState,
  Field,
  FieldGroup,
  PageHeader,
  Panel,
  SegmentedControl,
  StatusBanner,
  Tooltip,
} from "../components/Primitives";
import { parseYamlText, toYamlMappingText } from "../policies/policyUtils";
import { PolicySection } from "../policies/PolicyLayout";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type {
  GatewayConfig,
  McpConfig,
  McpFailureMode,
  McpPrefixMode,
  McpStatefulMode,
  McpTarget,
  McpTargetKind,
} from "../types";

const targetKinds: McpTargetKind[] = ["mcp", "sse", "stdio"];

type McpSettingsPatch = Partial<Omit<McpConfig, "gateways" | "port">> & {
  gateways?: McpConfig["gateways"] | null;
  port?: number | null;
};

export function McpServersPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const mcp = config.data?.mcp;
  const targets = useMemo(() => mcp?.targets ?? [], [mcp]);
  const [editing, setEditing] = useState<{
    previousName?: string;
    target: McpTarget;
  } | null>(null);
  const [deletingServer, setDeletingServer] = useState<string | null>(null);
  const [serverDrawer, setServerDrawer] = useStickyQueryParam("server");
  const linkedTarget =
    serverDrawer && serverDrawer !== "new" && serverDrawer !== "settings"
      ? targets.find((target) => target.name === serverDrawer)
      : null;
  const activeEditing =
    editing ??
    (serverDrawer === "new"
      ? { target: makeEmptyMcpTarget() }
      : linkedTarget
        ? {
            previousName: linkedTarget.name,
            target: structuredClone(linkedTarget),
          }
        : null);
  const settingsOpen = serverDrawer === "settings";

  function openNewServer() {
    setEditing(null);
    setServerDrawer("new");
  }

  function openEditServer(target: McpTarget) {
    setEditing(null);
    setServerDrawer(target.name);
  }

  function closeServerDrawer() {
    setEditing(null);
    setServerDrawer(null, "replace");
  }

  return (
    <div className="page-stack">
      <PageHeader
        title={tr("copy.mcpServers")}
        description={tr("copy.configureMcpTargetsServedByTheGateway")}
        actions={
          <div className="button-row">
            <button
              className="button"
              type="button"
              onClick={() => setServerDrawer("settings")}
            >
              <SlidersHorizontal size={16} />
              {tr("copy.settings")}
            </button>
            <button
              className="button primary"
              type="button"
              onClick={openNewServer}
            >
              <Plus size={16} />
              {tr("copy.addServer")}
            </button>
          </div>
        }
      />

      {update.isError && !activeEditing && !settingsOpen ? (
        <StatusBanner state="bad" title={tr("copy.saveFailed")}>
          {update.error.message}
        </StatusBanner>
      ) : null}
      {update.isSuccess ? (
        <StatusBanner state="ok" title={tr("copy.configurationSaved")} />
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title={tr("copy.loadingMcpServers")} />
        ) : config.isError ? (
          <StatusBanner
            state="bad"
            title={tr("copy.configurationApiUnavailable")}
          >
            {config.error.message}
          </StatusBanner>
        ) : targets.length === 0 ? (
          <EmptyState
            title={tr("copy.noMcpServersConfigured")}
            description={tr("copy.addATargetSoTheGatewayCanExposeMcpTraffic")}
            action={
              <button
                className="button primary"
                type="button"
                onClick={openNewServer}
              >
                <Server size={16} />
                {tr("copy.addServer")}
              </button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>{tr("copy.name")}</th>
                  <th>{tr("copy.type")}</th>
                  <th>{tr("copy.endpoint")}</th>
                  <th>{tr("copy.state")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {targets.map((target) => {
                  const kind = targetKind(target);
                  const warnings = targetWarnings(target);
                  return (
                    <tr key={target.name}>
                      <td className="strong">{target.name}</td>
                      <td>
                        <span className="badge">{transportLabel(kind)}</span>
                      </td>
                      <td>
                        <code>{targetEndpoint(target)}</code>
                      </td>
                      <td>
                        {warnings.length ? (
                          <span className="badge warn">
                            {warnings.length}
                            {tr("copy.warnings")}
                          </span>
                        ) : (
                          <span className="badge ok">{tr("copy.ready")}</span>
                        )}
                      </td>
                      <td className="row-actions">
                        <Tooltip content="Edit server">
                          <button
                            className="icon-button"
                            aria-label={tr("copy.editServer")}
                            type="button"
                            onClick={() => openEditServer(target)}
                          >
                            <Pencil size={16} />
                          </button>
                        </Tooltip>
                        <Tooltip content="Delete server">
                          <button
                            className="icon-button danger"
                            aria-label={tr("copy.deleteServer")}
                            type="button"
                            disabled={update.isPending}
                            onClick={() => setDeletingServer(target.name)}
                          >
                            <Trash2 size={16} />
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

      {activeEditing ? (
        <McpServerEditor
          key={activeEditing.previousName ?? "new"}
          initial={activeEditing.target}
          config={config.data}
          previousName={activeEditing.previousName}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onCancel={closeServerDrawer}
          onSave={(target, previousName) =>
            update.mutate(
              (next) => upsertMcpTarget(next, target, previousName),
              {
                onSuccess: closeServerDrawer,
              },
            )
          }
        />
      ) : null}
      {settingsOpen ? (
        <McpSettingsDrawer
          config={config.data}
          mcp={mcp}
          help={help}
          saving={update.isPending}
          saveError={update.isError ? update.error.message : null}
          onClose={closeServerDrawer}
          onSave={(settings) =>
            update.mutate(
              (next) => {
                Object.assign(ensureMcp(next), settings);
              },
              {
                onSuccess: closeServerDrawer,
              },
            )
          }
        />
      ) : null}
      {deletingServer ? (
        <ConfirmDialog
          title={tr("copy.deleteMcpServer")}
          destructive
          confirmLabel={tr("copy.deleteServer")}
          confirmDisabled={update.isPending}
          onCancel={() => setDeletingServer(null)}
          onConfirm={() =>
            update.mutate((next) => removeMcpTarget(next, deletingServer), {
              onSuccess: () => setDeletingServer(null),
            })
          }
        >
          <p>
            {tr("copy.delete")}
            <strong>{deletingServer}</strong>
            {tr("copy.trafficCanNoLongerBeSentToThisTarget")}
          </p>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

export function McpSettingsDrawer(props: {
  config?: GatewayConfig | null;
  mcp?: McpConfig | null;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onClose: () => void;
  onSave: (settings: McpSettingsPatch) => void;
}) {
  return (
    <Drawer title={tr("copy.settings")} onClose={props.onClose}>
      <McpSettings
        config={props.config}
        mcp={props.mcp}
        help={props.help}
        saving={props.saving}
        onSave={props.onSave}
      />
      {props.saveError ? (
        <StatusBanner state="bad" title={tr("copy.saveFailed")}>
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
  );
}

function McpSettings(props: {
  config?: GatewayConfig | null;
  mcp?: McpConfig | null;
  help: SchemaHelp;
  saving: boolean;
  onSave: (settings: McpSettingsPatch) => void;
}) {
  const [binding, setBinding] = useState<GatewayBindingValue>({
    gateways: props.mcp?.gateways ?? null,
    port: props.mcp?.port ?? null,
  });
  const [statefulMode, setStatefulMode] = useState<McpStatefulMode>(
    props.mcp?.statefulMode ?? "stateless",
  );
  const [prefixMode, setPrefixMode] = useState<McpPrefixMode | "none">(
    props.mcp?.prefixMode ?? "none",
  );
  const [failureMode, setFailureMode] = useState<McpFailureMode>(
    props.mcp?.failureMode ?? "failClosed",
  );
  const patch: McpSettingsPatch = {
    gateways: binding.gateways ?? null,
    port: binding.gateways ? null : (binding.port ?? null),
    statefulMode,
    prefixMode: prefixMode === "none" ? null : prefixMode,
    failureMode,
  };

  return (
    <form
      className="policy-editor-stack"
      onSubmit={(event) => {
        event.preventDefault();
        props.onSave(patch);
      }}
    >
      <PolicySection
        icon={<Server size={17} />}
        title={tr("copy.gatewayBinding")}
        description={tr("copy.chooseHowMcpIsExposed")}
      >
        <div className="form-grid">
          <GatewayBindingEditor
            config={props.config}
            value={binding}
            defaultPort={3000}
            portLabel="Port"
            portPlaceholder="3000"
            portTooltip={props.help.field<McpConfig>(
              "LocalSimpleMcpConfig",
              "port",
              "Gateway port for MCP traffic.",
            )}
            onChange={setBinding}
          />
        </div>
      </PolicySection>
      <PolicySection
        icon={<SlidersHorizontal size={17} />}
        title={tr("copy.mcpBehavior")}
        description={tr("copy.chooseSessionToolPrefixAndFailureBehavior")}
      >
        <div className="form-grid">
          <FieldGroup
            label={tr("copy.stateMode")}
            tooltip={props.help.field<McpConfig>(
              "LocalSimpleMcpConfig",
              "statefulMode",
              "Controls whether MCP sessions are preserved by the gateway.",
            )}
          >
            <EnumSelector
              ariaLabel="State mode"
              value={statefulMode}
              options={[
                {
                  value: "stateless",
                  label: tr("copy.stateless"),
                  description: tr(
                    "copy.doNotPreserveMcpSessionStateBetweenRequests",
                  ),
                },
                {
                  value: "stateful",
                  label: tr("copy.stateful"),
                  description: tr(
                    "copy.preserveMcpSessionsSoTargetsCanKeepPerSessionContext",
                  ),
                },
              ]}
              schema={props.help.node([
                "$defs",
                "LocalSimpleMcpConfig",
                "properties",
                "statefulMode",
              ])}
              onChange={setStatefulMode}
            />
          </FieldGroup>
          <FieldGroup
            label={tr("copy.prefixMode")}
            tooltip={props.help.field<McpConfig>(
              "LocalSimpleMcpConfig",
              "prefixMode",
              "Controls whether target names are prefixed when exposing tools.",
            )}
          >
            <EnumSelector
              ariaLabel="Prefix mode"
              value={prefixMode}
              options={[
                {
                  value: "none",
                  label: tr("copy.none_deku7v"),
                  description: tr(
                    "copy.exposeToolNamesWithoutAddingTheTargetName",
                  ),
                },
                {
                  value: "always",
                  label: tr("copy.always"),
                  description: tr(
                    "copy.alwaysPrefixExposedToolNamesWithTheTargetName",
                  ),
                },
                {
                  value: "conditional",
                  label: tr("copy.conditional"),
                  description: tr(
                    "copy.prefixOnlyWhenNeededToAvoidToolNameConflicts",
                  ),
                },
                {
                  value: "never",
                  label: tr("copy.never"),
                  description: tr(
                    "copy.neverPrefixCallsAreRoutedByToolNameWhichMustBeUniqueAcrossTargets",
                  ),
                },
              ]}
              schema={props.help.node([
                "$defs",
                "LocalSimpleMcpConfig",
                "properties",
                "prefixMode",
              ])}
              onChange={setPrefixMode}
            />
          </FieldGroup>
          <FieldGroup
            label={tr("copy.failureMode")}
            tooltip={props.help.field<McpConfig>(
              "LocalSimpleMcpConfig",
              "failureMode",
            )}
          >
            <EnumSelector
              ariaLabel="Failure mode"
              value={failureMode}
              options={[
                { value: "failClosed", label: tr("copy.failClosed") },
                { value: "failOpen", label: tr("copy.failOpen") },
              ]}
              schema={props.help.node(["$defs", "McpBackendFailureMode"])}
              onChange={setFailureMode}
            />
          </FieldGroup>
        </div>
      </PolicySection>
      <ConfigDiffSaveActions
        config={props.config}
        diffTitle="MCP settings config diff"
        saveLabel="Save settings"
        saving={props.saving}
        onSave={() => props.onSave(patch)}
        applyDiff={(next) => {
          Object.assign(ensureMcp(next), patch);
        }}
      />
    </form>
  );
}

function McpServerEditor(props: {
  initial: McpTarget;
  config?: GatewayConfig | null;
  previousName?: string;
  help: SchemaHelp;
  saving: boolean;
  saveError?: string | null;
  onCancel: () => void;
  onSave: (target: McpTarget, previousName?: string) => void;
}) {
  const [name, setName] = useState(props.initial.name);
  const [kind, setKind] = useState<McpTargetKind>(() => {
    const kind = targetKind(props.initial);
    return kind === "openapi" ? "mcp" : kind;
  });
  const network = networkTarget(props.initial);
  const stdio = "stdio" in props.initial ? props.initial.stdio : undefined;
  const [url, setUrl] = useState(() => networkUrl(network, kind));
  const [cmd, setCmd] = useState(stdio?.cmd ?? "");
  const [args, setArgs] = useState((stdio?.args ?? []).join(" "));
  const [envText, setEnvText] = useState(toYamlMappingText(stdio?.env));
  const [clearEnv, setClearEnv] = useState(Boolean(stdio?.clear_env));
  const [error, setError] = useState<string | null>(null);
  const draft = JSON.stringify({
    name,
    kind,
    url,
    cmd,
    args,
    envText,
    clearEnv,
  });
  const [initialDraft] = useState(() => draft);

  function targetPreview() {
    const base = {
      ...props.initial,
      name: name.trim(),
      policies: props.initial.policies,
    } as McpTarget;
    delete (base as Record<string, unknown>).mcp;
    delete (base as Record<string, unknown>).sse;
    delete (base as Record<string, unknown>).stdio;
    delete (base as Record<string, unknown>).openapi;
    if (kind === "stdio") {
      const env = envText.trim() ? parseEnvYaml(envText) : {};
      return {
        ...base,
        stdio: {
          cmd: cmd.trim(),
          args: splitArgs(args),
          env,
          clear_env: clearEnv,
        },
      };
    }
    const target = {
      host: url.trim() || null,
    };
    return kind === "sse" ? { ...base, sse: target } : { ...base, mcp: target };
  }

  function validTargetPreview() {
    try {
      setError(null);
      return targetPreview();
    } catch (err) {
      setError(
        err instanceof Error ? err.message : "Invalid server configuration",
      );
      return null;
    }
  }

  function save() {
    const target = validTargetPreview();
    if (!target) return;
    props.onSave(target, props.previousName);
  }

  return (
    <Drawer
      title={props.previousName ? "Edit MCP server" : "Add MCP server"}
      onClose={props.onCancel}
      dirty={draft !== initialDraft}
      saving={props.saving}
      footer={(requestClose) => (
        <ConfigDiffSaveActions
          config={props.config}
          diffTitle="MCP server config diff"
          saveLabel="Save server"
          saving={props.saving}
          saveDisabled={!name.trim() || (kind === "stdio" && !cmd.trim())}
          onCancel={requestClose}
          onSave={save}
          beforeDiff={() => Boolean(validTargetPreview())}
          applyDiff={(next) => {
            const target = targetPreview();
            upsertMcpTarget(next, target, props.previousName);
          }}
        />
      )}
    >
      <div className="form-grid">
        <Field
          label={tr("copy.serverName")}
          tooltip={props.help.field<McpTarget>(
            "LocalMcpTarget",
            "name",
            "Name used to identify this MCP target.",
          )}
        >
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="weather"
          />
        </Field>
      </div>
      <FieldGroup
        label={tr("copy.transport")}
        tooltip={tr("copy.howTheGatewayConnectsToThisMcpTarget")}
      >
        <SegmentedControl
          ariaLabel="Transport"
          value={kind}
          className="mcp-transport-control"
          options={targetKinds.map((value) => ({
            value,
            label: transportLabel(value),
          }))}
          onChange={(value) => {
            setKind(value);
            if (!url.trim())
              setUrl(
                value === "sse"
                  ? "http://localhost:3001/sse"
                  : "http://localhost:3001/mcp",
              );
          }}
        />
      </FieldGroup>

      {kind === "stdio" ? (
        <>
          <Field
            label={tr("copy.command")}
            tooltip={props.help.field<McpTarget>(
              "LocalMcpTarget1",
              "stdio.cmd",
              "Command to launch for command-line MCP servers.",
            )}
          >
            <input
              value={cmd}
              onChange={(event) => setCmd(event.target.value)}
              placeholder="npx"
            />
          </Field>
          <Field
            label={tr("copy.arguments")}
            tooltip={props.help.field<McpTarget>(
              "LocalMcpTarget1",
              "stdio.args",
              "Command arguments passed to the MCP server process.",
            )}
          >
            <input
              value={args}
              onChange={(event) => setArgs(event.target.value)}
              placeholder={tr("copy.yModelcontextprotocolServerFilesystemTmp")}
            />
          </Field>
          <FieldGroup
            label={tr("copy.environmentYaml")}
            tooltip={props.help.field<McpTarget>(
              "LocalMcpTarget1",
              "stdio.env",
              "Environment variables set for the MCP server process.",
            )}
          >
            <MiniMonacoEditor
              language="yaml"
              value={envText}
              onChange={setEnvText}
            />
          </FieldGroup>
          <label className="toggle-row">
            <input
              type="checkbox"
              checked={clearEnv}
              onChange={(event) => setClearEnv(event.target.checked)}
            />
            {tr("copy.clearEnvironment")}
          </label>
        </>
      ) : (
        <Field
          label={tr("copy.url")}
          tooltip={
            kind === "sse"
              ? props.help.field<McpTarget>(
                  "LocalMcpTarget1",
                  "sse.host",
                  "URL of the MCP server endpoint.",
                )
              : props.help.field<McpTarget>(
                  "LocalMcpTarget1",
                  "mcp.host",
                  "URL of the MCP server endpoint.",
                )
          }
        >
          <input
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            placeholder={
              kind === "sse"
                ? "http://localhost:3001/sse"
                : "http://localhost:3001/mcp"
            }
          />
        </Field>
      )}
      {error ? (
        <StatusBanner state="bad" title={tr("copy.invalidServer")}>
          {error}
        </StatusBanner>
      ) : null}
      {props.saveError ? (
        <StatusBanner state="bad" title={tr("copy.saveFailed")}>
          {props.saveError}
        </StatusBanner>
      ) : null}
    </Drawer>
  );
}

function targetKind(target: McpTarget): McpTargetKind {
  if ("sse" in target) return "sse";
  if ("stdio" in target) return "stdio";
  if ("openapi" in target) return "openapi";
  return "mcp";
}

function networkTarget(target: McpTarget) {
  if ("sse" in target) return target.sse;
  if ("mcp" in target) return target.mcp;
  if ("openapi" in target) return target.openapi;
  return undefined;
}

function targetEndpoint(target: McpTarget) {
  if ("stdio" in target) return stdioCommandLine(target.stdio);
  const network = networkTarget(target);
  if (!network) return "n/a";
  const host = network.host ?? "localhost";
  const port = network.port ? `:${network.port}` : "";
  const path = network.path ?? "";
  return `${host}${port}${path}`;
}

function stdioCommandLine(stdio: { cmd: string; args?: string[] }) {
  const parts = [stdio.cmd, ...(stdio.args ?? [])].filter((part) =>
    part.trim(),
  );
  return parts.map(shellDisplayArg).join(" ");
}

function shellDisplayArg(value: string) {
  return /\s/.test(value) ? JSON.stringify(value) : value;
}

function targetWarnings(target: McpTarget) {
  const warnings: string[] = [];
  if (!target.name.trim()) warnings.push("Server name is required.");
  if ("stdio" in target && !target.stdio.cmd.trim())
    warnings.push("Command is required.");
  if (!("stdio" in target)) {
    const network = networkTarget(target);
    if (!network?.host) warnings.push("URL should be set.");
  }
  return warnings;
}

function splitArgs(value: string) {
  return value.trim() ? value.trim().split(/\s+/) : [];
}

function parseEnvYaml(value: string) {
  const parsed = parseYamlText(value);
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(tr("copy.environmentMustBeAYamlMapping"));
  }
  return Object.fromEntries(
    Object.entries(parsed).map(([key, item]) => [key, String(item)]),
  );
}

function transportLabel(kind: McpTargetKind) {
  if (kind === "mcp") return "Streamable HTTP";
  if (kind === "sse") return "Legacy SSE";
  if (kind === "stdio") return "Command Line";
  return "OpenAPI";
}

function networkUrl(
  network: ReturnType<typeof networkTarget>,
  kind: McpTargetKind,
) {
  if (!network)
    return kind === "sse"
      ? "http://localhost:3001/sse"
      : "http://localhost:3001/mcp";
  if (
    network.host?.startsWith("http://") ||
    network.host?.startsWith("https://")
  )
    return network.host;
  const host = network.host ?? "localhost";
  const port = network.port ? `:${network.port}` : "";
  const path = network.path ?? (kind === "sse" ? "/sse" : "/mcp");
  return `http://${host}${port}${path}`;
}
