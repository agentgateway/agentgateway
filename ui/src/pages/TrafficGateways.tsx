import { GitBranch, Network, Pencil, Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import {
  Drawer,
  Dropdown,
  EmptyState,
  Field,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import {
  ConfigDiffDrawer,
  ConfigDiffSaveActions,
  configDiffText,
} from "../components/ConfigDiffDrawer";
import { isDatabaseConfigResource } from "../config";
import { useStickyQueryParam } from "../drawerRouteState";
import {
  useDeleteConfigResource,
  useMcpConfigData,
  useTrafficConfigData,
  useUpdateConfig,
  useUpsertConfigResource,
} from "../hooks";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type {
  GatewayConfig,
  TrafficBind,
  TrafficGateway,
  TrafficGatewayListener,
  TrafficListener,
  TrafficRoute,
  TrafficTcpRoute,
} from "../types";
import type { LocalTLSServerConfig } from "../gateway-config";
import type {
  LocalAttachedRoute,
  LocalAttachedTCPRoute,
} from "../gateway-config";
import { TrafficPolicySection } from "./traffic/TrafficPolicySection";

type GatewayRow = {
  name: string;
  gateway: TrafficGateway;
};

type GatewayEditorState = {
  previousName?: string;
  name: string;
  gateway: TrafficGateway;
};

type GatewayListenerEditorState = {
  gatewayName: string;
  listenerIndex?: number;
  listener: TrafficGatewayListener;
};

type GatewayProtocol = "HTTP" | "HTTPS" | "TCP" | "TLS";

const gatewayProtocolOptions = [
  { value: "HTTP", label: "HTTP" },
  { value: "HTTPS", label: "HTTPS" },
  { value: "TCP", label: "TCP" },
  { value: "TLS", label: "TLS" },
];

export function TrafficGatewaysPage() {
  const traffic = useTrafficConfigData();
  const mcpData = useMcpConfigData();
  const config = {
    ...traffic.config,
    data:
      traffic.data && mcpData.data?.mcp
        ? { ...traffic.data, mcp: mcpData.data.mcp }
        : traffic.data,
    isLoading: traffic.isLoading || mcpData.isLoading,
    isError: Boolean(traffic.error ?? mcpData.error),
    error: traffic.error ?? mcpData.error,
  };
  const update = useUpdateConfig();
  const upsertResource = useUpsertConfigResource();
  const deleteResource = useDeleteConfigResource();
  const help = useSchemaHelp();
  const [drawer, setDrawer] = useStickyQueryParam("gateway");
  const [migrationOpen, setMigrationOpen] = useState(false);
  const gateways = useMemo<GatewayRow[]>(
    () =>
      Object.entries(config.data?.gateways ?? {}).map(([name, gateway]) => ({
        name,
        gateway,
      })),
    [config.data],
  );
  const hasLegacyBinds = Boolean(config.data?.binds?.length);
  const showLegacyBindsWarning = hasLegacyBinds && gateways.length === 0;
  const migration = useMemo(() => bindMigration(config.data), [config.data]);
  const activeGateway =
    drawer === "new"
      ? { name: "public", gateway: { port: 8080 } }
      : drawer && !drawer.startsWith("listener:")
        ? gatewayEditorFromName(drawer, config.data?.gateways)
        : null;
  const activeListener = gatewayListenerEditorFromValue(
    drawer,
    config.data?.gateways,
  );

  function closeDrawer() {
    setDrawer(null, "replace");
  }

  function saveGateway(draft: GatewayEditorState) {
    upsertResource.mutate(
      {
        kind: "traffic.gateway",
        value: { name: draft.name, ...cleanGateway(draft.gateway) },
        previousId: draft.previousName,
      },
      { onSuccess: closeDrawer },
    );
  }

  function deleteGateway(name: string) {
    deleteResource.mutate({ kind: "traffic.gateway", id: name });
  }

  function databaseGateway(name: string) {
    return isDatabaseConfigResource(traffic.resources, "traffic.gateway", name);
  }

  function saveGatewayListeners(
    gatewayName: string,
    updateListeners: (gateway: TrafficGateway) => void,
  ) {
    const gateway = structuredClone(config.data?.gateways?.[gatewayName]);
    if (!gateway) return;
    updateListeners(gateway);
    upsertResource.mutate(
      {
        kind: "traffic.gateway",
        value: { name: gatewayName, ...cleanGateway(gateway) },
        previousId: gatewayName,
      },
      { onSuccess: closeDrawer },
    );
  }

  const saving =
    update.isPending || upsertResource.isPending || deleteResource.isPending;
  const saveError =
    update.error?.message ??
    upsertResource.error?.message ??
    deleteResource.error?.message ??
    null;
  const saved =
    update.isSuccess || upsertResource.isSuccess || deleteResource.isSuccess;

  return (
    <div className="page-stack">
      <PageHeader
        title="Traffic Gateways"
        description="Configure named gateway listeners that LLM, MCP, UI, and routes can attach to."
        actions={
          <button
            className="button primary"
            type="button"
            onClick={() => setDrawer("new")}
          >
            <Plus size={16} />
            Add gateway
          </button>
        }
      />

      {saveError ? (
        <StatusBanner state="bad" title="Save failed">
          {saveError}
        </StatusBanner>
      ) : null}
      {saved ? <StatusBanner state="ok" title="Configuration saved" /> : null}
      {showLegacyBindsWarning ? (
        <StatusBanner
          state="warn"
          title="Detected legacy binds config"
          action={
            migration ? (
              <button
                className="button"
                type="button"
                onClick={() => setMigrationOpen(true)}
              >
                <GitBranch size={16} />
                Review migration
              </button>
            ) : null
          }
        >
          This configuration uses legacy <code>binds</code> and has no{" "}
          <code>gateways</code>. Consider moving listener ownership to{" "}
          <code>gateways</code>.
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading gateways" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {config.error?.message}
          </StatusBanner>
        ) : gateways.length === 0 ? (
          <EmptyState
            title="No gateways configured"
            description="Add a named gateway before attaching LLM, MCP, UI, or routes."
            action={
              <button
                className="button primary"
                type="button"
                onClick={() => setDrawer("new")}
              >
                <Network size={16} />
                Add gateway
              </button>
            }
          />
        ) : (
          <div className="traffic-bind-list">
            {gateways.map(({ name, gateway }) => (
              <section className="traffic-bind" key={name}>
                <div className="traffic-bind-header">
                  <div>
                    <h3>{name}</h3>
                    <p>
                      Port {gatewayPortLabel(gateway)}
                      {gateway.listeners?.length
                        ? `, ${gateway.listeners.length} named listeners`
                        : ""}
                      , {gatewayPolicyCount(gateway)} policies
                    </p>
                  </div>
                  <div className="button-row">
                    <span className="badge">
                      {gatewayProtocolLabel(gateway)}
                    </span>
                    <span
                      className={gatewayHasTls(gateway) ? "badge ok" : "badge"}
                    >
                      {gatewayHasTls(gateway) ? "TLS" : "Plain"}
                    </span>
                    {gateway.listeners?.length ? (
                      <Tooltip content="Add listener">
                        <button
                          className="icon-button"
                          type="button"
                          aria-label="Add listener"
                          disabled={traffic.hybrid && !databaseGateway(name)}
                          onClick={() => setDrawer(`listener:new:${name}`)}
                        >
                          <Plus size={16} />
                        </button>
                      </Tooltip>
                    ) : null}
                    <Tooltip
                      content={
                        traffic.hybrid && !databaseGateway(name)
                          ? "File-owned gateways must be edited in raw configuration"
                          : "Edit gateway"
                      }
                    >
                      <button
                        className="icon-button"
                        type="button"
                        aria-label="Edit gateway"
                        disabled={traffic.hybrid && !databaseGateway(name)}
                        onClick={() => setDrawer(name)}
                      >
                        <Pencil size={16} />
                      </button>
                    </Tooltip>
                    <Tooltip
                      content={
                        traffic.hybrid && !databaseGateway(name)
                          ? "File-owned gateways cannot be deleted here"
                          : "Delete gateway"
                      }
                    >
                      <button
                        className="icon-button danger"
                        type="button"
                        aria-label="Delete gateway"
                        disabled={traffic.hybrid && !databaseGateway(name)}
                        onClick={() => deleteGateway(name)}
                      >
                        <Trash2 size={16} />
                      </button>
                    </Tooltip>
                  </div>
                </div>
                {gateway.listeners?.length ? (
                  <div className="table-wrap">
                    <table>
                      <thead>
                        <tr>
                          <th>Name</th>
                          <th>Hostname</th>
                          <th>Protocol</th>
                          <th>TLS</th>
                          <th>Policies</th>
                          <th />
                        </tr>
                      </thead>
                      <tbody>
                        {gateway.listeners.map((listener, listenerIndex) => (
                          <tr key={`${listener.name}-${listenerIndex}`}>
                            <td className="strong">
                              {gatewayListenerName(listener, listenerIndex)}
                            </td>
                            <td>{listener.hostname || "*"}</td>
                            <td>{gatewayProtocolLabel(listener)}</td>
                            <td>
                              <span
                                className={listener.tls ? "badge ok" : "badge"}
                              >
                                {listener.tls ? "TLS" : "Plain"}
                              </span>
                            </td>
                            <td>{gatewayListenerPolicyCount(listener)}</td>
                            <td className="row-actions">
                              <Tooltip
                                content={
                                  traffic.hybrid && !databaseGateway(name)
                                    ? "File-owned listeners must be edited in raw configuration"
                                    : "Edit listener"
                                }
                              >
                                <button
                                  className="icon-button"
                                  type="button"
                                  aria-label="Edit listener"
                                  disabled={
                                    traffic.hybrid && !databaseGateway(name)
                                  }
                                  onClick={() =>
                                    setDrawer(
                                      `listener:edit:${name}:${listenerIndex}`,
                                    )
                                  }
                                >
                                  <Pencil size={16} />
                                </button>
                              </Tooltip>
                              <Tooltip
                                content={
                                  traffic.hybrid && !databaseGateway(name)
                                    ? "File-owned listeners cannot be deleted here"
                                    : "Delete listener"
                                }
                              >
                                <button
                                  className="icon-button danger"
                                  type="button"
                                  aria-label="Delete listener"
                                  disabled={
                                    traffic.hybrid && !databaseGateway(name)
                                  }
                                  onClick={() =>
                                    saveGatewayListeners(name, (gateway) => {
                                      if (!gateway.listeners) return;
                                      gateway.listeners =
                                        gateway.listeners.filter(
                                          (_, index) => index !== listenerIndex,
                                        );
                                      if (gateway.listeners.length === 0)
                                        delete gateway.listeners;
                                    })
                                  }
                                >
                                  <Trash2 size={16} />
                                </button>
                              </Tooltip>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                ) : null}
              </section>
            ))}
          </div>
        )}
      </Panel>

      {activeGateway ? (
        <GatewayEditor
          key={`${activeGateway.previousName ?? "new"}:${drawer ?? ""}`}
          initial={activeGateway}
          config={config.data}
          help={help}
          saving={saving}
          databaseBacked={
            traffic.hybrid &&
            (!activeGateway.previousName ||
              isDatabaseConfigResource(
                traffic.resources,
                "traffic.gateway",
                activeGateway.previousName,
              ))
          }
          onCancel={closeDrawer}
          onSave={saveGateway}
        />
      ) : null}

      {activeListener ? (
        <GatewayListenerEditor
          key={drawer ?? "listener-local"}
          editing={activeListener}
          config={config.data}
          help={help}
          saving={saving}
          databaseBacked={
            traffic.hybrid &&
            isDatabaseConfigResource(
              traffic.resources,
              "traffic.gateway",
              activeListener.gatewayName,
            )
          }
          onCancel={closeDrawer}
          onSave={(gatewayName, listener, listenerIndex) =>
            saveGatewayListeners(gatewayName, (gateway) => {
              gateway.listeners ??= [];
              if (typeof listenerIndex === "number") {
                gateway.listeners[listenerIndex] = listener;
              } else {
                gateway.listeners.push(listener);
              }
            })
          }
        />
      ) : null}

      {migrationOpen && config.data && migration ? (
        <ConfigDiffDrawer
          title="Migrate binds to gateways"
          {...configDiffText(config.data, migration.config)}
          saving={update.isPending}
          onClose={() => setMigrationOpen(false)}
          onSave={() =>
            update.mutate(
              (next) => {
                applyBindMigration(next);
              },
              {
                onSuccess: () => setMigrationOpen(false),
              },
            )
          }
        />
      ) : null}
    </div>
  );
}

function GatewayEditor(props: {
  initial: GatewayEditorState;
  config?: GatewayConfig;
  help: SchemaHelp;
  saving: boolean;
  databaseBacked?: boolean;
  onCancel: () => void;
  onSave: (draft: GatewayEditorState) => void;
}) {
  const [name, setName] = useState(props.initial.name);
  const [defaultGateway, setDefaultGateway] = useState(
    props.initial.name === "default",
  );
  const [gateway, setGateway] = useState<TrafficGateway>(
    structuredClone(props.initial.gateway),
  );
  const [multipleListeners, setMultipleListeners] = useState(
    Boolean(gateway.listeners?.length),
  );
  const [cert, setCert] = useState(gateway.tls?.cert ?? "");
  const [key, setKey] = useState(gateway.tls?.key ?? "");
  const policyCount = gatewayPolicyCount(gateway);
  const protocol = effectiveGatewayProtocol(gateway);
  const hasTls = protocol === "HTTPS" || protocol === "TLS";
  const canEnableMultipleListeners = !hasTls && policyCount === 0;
  const anotherDefaultGateway = Boolean(
    props.config?.gateways?.default && props.initial.previousName !== "default",
  );
  const defaultTraffic = defaultGatewayTraffic(props.config);
  const invalidDefaultName = !defaultGateway && name.trim() === "default";
  const duplicateName = Boolean(
    name.trim() &&
    name.trim() !== props.initial.previousName &&
    props.config?.gateways?.[name.trim()],
  );
  const preview: TrafficGateway = cleanGateway({
    ...(multipleListeners ? withoutGatewayPolicies(gateway) : gateway),
    listeners: multipleListeners
      ? gateway.listeners?.length
        ? gateway.listeners
        : [{ name: "listener1" }]
      : [],
    tls:
      !multipleListeners &&
      (protocol === "HTTPS" || protocol === "TLS") &&
      (cert.trim() || key.trim())
        ? { ...(gateway.tls ?? {}), cert: cert.trim(), key: key.trim() }
        : null,
  });

  return (
    <Drawer
      title={props.initial.previousName ? "Edit gateway" : "Add gateway"}
      onClose={props.onCancel}
      footer={
        <ConfigDiffSaveActions
          config={props.config}
          resourceDiff={
            props.databaseBacked
              ? {
                  original: {
                    name: props.initial.name,
                    ...props.initial.gateway,
                  },
                  modified: { name: name.trim(), ...preview },
                }
              : undefined
          }
          diffTitle="Gateway config diff"
          saveLabel="Save gateway"
          saving={props.saving}
          saveDisabled={!name.trim() || invalidDefaultName || duplicateName}
          onCancel={props.onCancel}
          onSave={() =>
            props.onSave({
              previousName: props.initial.previousName,
              name: name.trim(),
              gateway: preview,
            })
          }
          applyDiff={(next) => {
            next.gateways ??= {};
            if (
              props.initial.previousName &&
              props.initial.previousName !== name.trim()
            ) {
              delete next.gateways[props.initial.previousName];
            }
            next.gateways[name.trim()] = cleanGateway(preview);
          }}
        />
      }
    >
      <div className="form-grid gateway-identity-grid">
        <Field
          label="Name"
          tooltip="Features and routes reference this gateway by name."
        >
          <input
            value={name}
            disabled={defaultGateway}
            onChange={(event) => setName(event.target.value)}
            placeholder="public"
          />
        </Field>
        <Field
          label="Port"
          tooltip={props.help.field<TrafficGateway>("LocalGateway", "port")}
        >
          <input
            value={gateway.port?.toString() ?? ""}
            onChange={(event) =>
              setGateway({
                ...gateway,
                port: parsePort(event.target.value),
              })
            }
            placeholder="443"
          />
        </Field>
      </div>

      <div className="gateway-default-option">
        <label className="config-option-row">
          <input
            type="checkbox"
            checked={defaultGateway}
            disabled={anotherDefaultGateway}
            onChange={(event) => {
              const checked = event.target.checked;
              setDefaultGateway(checked);
              setName(
                checked
                  ? "default"
                  : availableGatewayName(
                      props.config?.gateways,
                      props.initial.previousName,
                    ),
              );
            }}
          />
          <span>
            <strong>Default gateway</strong>
            <small>
              {anotherDefaultGateway
                ? "Another gateway is already the default gateway."
                : "Use this gateway for enabled traffic without an explicit gateway selection."}
            </small>
          </span>
        </label>

        {invalidDefaultName ? (
          <StatusBanner state="bad" title="Default name is reserved">
            Select Default gateway to use the name <code>default</code>.
          </StatusBanner>
        ) : duplicateName ? (
          <StatusBanner state="bad" title="Gateway name already exists">
            Choose a unique gateway name.
          </StatusBanner>
        ) : null}

        {defaultGateway && props.initial.previousName !== "default" ? (
          <StatusBanner state="warn" title="Default gateway">
            This will make the gateway the default gateway. This will impact{" "}
            {defaultTraffic.length ? defaultTraffic.join(", ") : "no enabled"}{" "}
            traffic.
          </StatusBanner>
        ) : null}
      </div>

      {!multipleListeners ? (
        <div className="form-grid">
          <Field
            label="Protocol"
            tooltip={props.help.field<TrafficGateway>(
              "LocalGateway",
              "protocol",
            )}
          >
            <Dropdown
              ariaLabel="Protocol"
              value={protocol}
              options={gatewayProtocolOptions}
              onChange={(value) => {
                const nextProtocol = value as GatewayProtocol;
                setGateway({
                  ...gateway,
                  protocol: nextProtocol,
                  tls:
                    nextProtocol === "HTTPS" || nextProtocol === "TLS"
                      ? gateway.tls
                      : null,
                });
                if (nextProtocol === "HTTP" || nextProtocol === "TCP") {
                  setCert("");
                  setKey("");
                }
              }}
            />
          </Field>
        </div>
      ) : null}

      <div className="form-grid">
        <label className="config-option-row">
          <input
            type="checkbox"
            checked={multipleListeners}
            disabled={!multipleListeners && !canEnableMultipleListeners}
            onChange={(event) => {
              const enabled = event.target.checked;
              setMultipleListeners(enabled);
              if (enabled) {
                setGateway((current) => ({
                  ...withoutGatewayPolicies(current),
                  protocol: null,
                  tls: null,
                  listeners: current.listeners?.length
                    ? current.listeners
                    : [{ name: "listener1" }],
                }));
                setCert("");
                setKey("");
              } else {
                setGateway((current) => ({ ...current, listeners: [] }));
              }
            }}
          />
          <span>
            <strong>Multiple listeners</strong>
            <small>
              {!multipleListeners && !canEnableMultipleListeners
                ? "Unavailable while gateway TLS or policies are configured."
                : "Use named listeners for per-hostname TLS and policies."}
            </small>
          </span>
        </label>
      </div>

      {!multipleListeners ? (
        protocol === "HTTPS" || protocol === "TLS" ? (
          <GatewayTLSFields
            cert={cert}
            keyValue={key}
            help={props.help}
            onCertChange={setCert}
            onKeyChange={setKey}
          />
        ) : null
      ) : null}

      {!multipleListeners ? (
        <TrafficPolicySection
          title="Gateway policies"
          schemaRoot="LocalGatewayPolicy"
          policies={gatewayPolicies(gateway)}
          onChange={(policies) =>
            setGateway({ ...withoutGatewayPolicies(gateway), ...policies })
          }
        />
      ) : null}

      <details open>
        <summary>Resulting YAML</summary>
        <YamlBlock value={{ [name.trim() || "gateway"]: preview }} />
      </details>
    </Drawer>
  );
}

function defaultGatewayTraffic(config: GatewayConfig | undefined) {
  const traffic: string[] = [];
  if (config?.llm) traffic.push("LLM");
  if (config && Object.prototype.hasOwnProperty.call(config, "ui")) {
    traffic.push("UI");
  }
  if (config?.mcp) traffic.push("MCP");
  return traffic;
}

function availableGatewayName(
  gateways: Record<string, TrafficGateway> | undefined,
  currentName: string | undefined,
) {
  const names = new Set(Object.keys(gateways ?? {}));
  if (currentName) names.delete(currentName);
  if (!names.has("public")) return "public";
  let suffix = 2;
  while (names.has(`public-${suffix}`)) suffix += 1;
  return `public-${suffix}`;
}

function GatewayTLSFields(props: {
  cert: string;
  keyValue: string;
  help: SchemaHelp;
  onCertChange: (value: string) => void;
  onKeyChange: (value: string) => void;
}) {
  return (
    <details>
      <summary>TLS</summary>
      <div className="form-grid">
        <Field
          label="Certificate"
          tooltip={props.help.field<LocalTLSServerConfig>(
            "LocalTLSServerConfig",
            "cert",
          )}
        >
          <input
            value={props.cert}
            onChange={(event) => props.onCertChange(event.target.value)}
            placeholder="/etc/certs/tls.crt"
          />
        </Field>
        <Field
          label="Key"
          tooltip={props.help.field<LocalTLSServerConfig>(
            "LocalTLSServerConfig",
            "key",
          )}
        >
          <input
            value={props.keyValue}
            onChange={(event) => props.onKeyChange(event.target.value)}
            placeholder="/etc/certs/tls.key"
          />
        </Field>
      </div>
    </details>
  );
}

function GatewayListenerEditor(props: {
  editing: GatewayListenerEditorState;
  config?: GatewayConfig;
  help: SchemaHelp;
  saving: boolean;
  databaseBacked?: boolean;
  onCancel: () => void;
  onSave: (
    gatewayName: string,
    listener: TrafficGatewayListener,
    listenerIndex?: number,
  ) => void;
}) {
  const [listener, setListener] = useState<TrafficGatewayListener>(
    structuredClone(props.editing.listener),
  );
  const [cert, setCert] = useState(listener.tls?.cert ?? "");
  const [key, setKey] = useState(listener.tls?.key ?? "");
  const protocol = effectiveGatewayProtocol(listener);
  const preview = cleanGatewayListener({
    ...listener,
    tls:
      (protocol === "HTTPS" || protocol === "TLS") &&
      (cert.trim() || key.trim())
        ? { ...(listener.tls ?? {}), cert: cert.trim(), key: key.trim() }
        : null,
  });
  const originalGateway = props.config?.gateways?.[props.editing.gatewayName];
  const modifiedGateway = originalGateway
    ? structuredClone(originalGateway)
    : undefined;
  if (modifiedGateway) {
    modifiedGateway.listeners ??= [];
    if (typeof props.editing.listenerIndex === "number") {
      modifiedGateway.listeners[props.editing.listenerIndex] = preview;
    } else {
      modifiedGateway.listeners.push(preview);
    }
  }

  return (
    <Drawer
      title={
        typeof props.editing.listenerIndex === "number"
          ? "Edit listener"
          : "Add listener"
      }
      onClose={props.onCancel}
      footer={
        <ConfigDiffSaveActions
          config={props.config}
          resourceDiff={
            props.databaseBacked && originalGateway && modifiedGateway
              ? {
                  original: {
                    name: props.editing.gatewayName,
                    ...originalGateway,
                  },
                  modified: {
                    name: props.editing.gatewayName,
                    ...modifiedGateway,
                  },
                }
              : undefined
          }
          diffTitle="Gateway listener config diff"
          saveLabel="Save listener"
          saving={props.saving}
          onCancel={props.onCancel}
          onSave={() =>
            props.onSave(
              props.editing.gatewayName,
              preview,
              props.editing.listenerIndex,
            )
          }
          applyDiff={(next) => {
            const gateway = next.gateways?.[props.editing.gatewayName];
            if (!gateway) return;
            gateway.listeners ??= [];
            if (typeof props.editing.listenerIndex === "number") {
              gateway.listeners[props.editing.listenerIndex] = preview;
            } else {
              gateway.listeners.push(preview);
            }
          }}
        />
      }
    >
      <div className="form-grid">
        <Field
          label="Name"
          tooltip={props.help.field<TrafficGatewayListener>(
            "LocalGatewayListener",
            "name",
          )}
        >
          <input
            value={listener.name ?? ""}
            onChange={(event) =>
              setListener({ ...listener, name: event.target.value || null })
            }
            placeholder="listener1"
          />
        </Field>
        <Field
          label="Protocol"
          tooltip={props.help.field<TrafficGatewayListener>(
            "LocalGatewayListener",
            "protocol",
          )}
        >
          <Dropdown
            ariaLabel="Protocol"
            value={protocol}
            options={gatewayProtocolOptions}
            onChange={(value) => {
              const nextProtocol = value as GatewayProtocol;
              setListener({
                ...listener,
                protocol: nextProtocol,
                tls:
                  nextProtocol === "HTTPS" || nextProtocol === "TLS"
                    ? listener.tls
                    : null,
              });
              if (nextProtocol === "HTTP" || nextProtocol === "TCP") {
                setCert("");
                setKey("");
              }
            }}
          />
        </Field>
        <Field
          label="Hostname"
          tooltip={props.help.field<TrafficGatewayListener>(
            "LocalGatewayListener",
            "hostname",
            "Can be an exact hostname or wildcard. Leave blank to match all hostnames.",
          )}
        >
          <input
            value={listener.hostname ?? ""}
            onChange={(event) =>
              setListener({ ...listener, hostname: event.target.value || null })
            }
            placeholder="*"
          />
        </Field>
      </div>
      {protocol === "HTTPS" || protocol === "TLS" ? (
        <GatewayTLSFields
          cert={cert}
          keyValue={key}
          help={props.help}
          onCertChange={setCert}
          onKeyChange={setKey}
        />
      ) : null}
      <TrafficPolicySection
        title="Listener policies"
        schemaRoot="LocalGatewayPolicy"
        policies={gatewayListenerPolicies(listener)}
        onChange={(policies) =>
          setListener({
            ...withoutGatewayListenerPolicies(listener),
            ...policies,
          })
        }
      />
      <details open>
        <summary>Resulting YAML</summary>
        <YamlBlock value={preview} />
      </details>
    </Drawer>
  );
}

function gatewayEditorFromName(
  name: string,
  gateways: Record<string, TrafficGateway> | undefined,
): GatewayEditorState | null {
  const gateway = gateways?.[name];
  return gateway
    ? { previousName: name, name, gateway: structuredClone(gateway) }
    : null;
}

function gatewayListenerEditorFromValue(
  value: string | null,
  gateways: Record<string, TrafficGateway> | undefined,
): GatewayListenerEditorState | null {
  if (!value?.startsWith("listener:")) return null;
  const parts = value.split(":");
  const [, action, gatewayName, index] = parts;
  const gateway = gateways?.[gatewayName];
  if (!gateway) return null;
  if (action === "new" && parts.length === 3) {
    return {
      gatewayName,
      listener: {
        name: `listener${(gateway.listeners?.length ?? 0) + 1}`,
      },
    };
  }
  if (action !== "edit" || parts.length !== 4) return null;
  const listenerIndex = Number(index);
  if (!Number.isInteger(listenerIndex)) return null;
  const listener = gateway.listeners?.[listenerIndex];
  return listener
    ? {
        gatewayName,
        listenerIndex,
        listener: structuredClone(listener),
      }
    : null;
}

function parsePort(value: string) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : null;
}

function cleanGateway(gateway: TrafficGateway): TrafficGateway {
  const next = { ...gateway };
  if (next.listeners?.length) {
    next.listeners = next.listeners.map(cleanGatewayListener);
  } else {
    delete next.listeners;
  }
  if (!next.port) delete next.port;
  if (!next.tls) delete next.tls;
  cleanProtocol(next);
  return Object.fromEntries(
    Object.entries(next).filter(
      ([, value]) => value !== null && value !== undefined,
    ),
  ) as TrafficGateway;
}

function cleanGatewayListener(
  listener: TrafficGatewayListener,
): TrafficGatewayListener {
  const { port: _port, ...next } = listener as TrafficGatewayListener & {
    port?: number | null;
  };
  cleanProtocol(next);
  return Object.fromEntries(
    Object.entries(next).filter(
      ([, value]) => value !== null && value !== undefined,
    ),
  ) as TrafficGatewayListener;
}

function cleanProtocol(
  value: (TrafficGateway | TrafficGatewayListener) & {
    protocol?: GatewayProtocol | null;
  },
) {
  if (value.protocol === "HTTP" && !value.tls) delete value.protocol;
  if (value.protocol === "HTTPS" && value.tls) delete value.protocol;
}

function effectiveGatewayProtocol(
  value: (TrafficGateway | TrafficGatewayListener) & {
    protocol?: GatewayProtocol | null;
  },
): GatewayProtocol {
  return value.protocol ?? (value.tls ? "HTTPS" : "HTTP");
}

function bindMigration(config: GatewayConfig | null | undefined) {
  if (!config?.binds?.length) return null;
  const next = structuredClone(config);
  const convertedListeners = applyBindMigration(next);
  return convertedListeners > 0 ? { config: next, convertedListeners } : null;
}

function applyBindMigration(config: GatewayConfig) {
  if (!config.binds?.length) return 0;
  config.gateways ??= {};
  const remainingBinds: TrafficBind[] = [];
  let convertedListeners = 0;
  const migratableBinds = config.binds.filter((bind) => {
    if (!bindIsMigratable(bind)) return false;
    const convertible = bind.listeners.filter(listenerIsMigratable);
    return (
      convertible.length > 0 &&
      convertible.length === bind.listeners.length &&
      hasOneRouteKind(convertible) &&
      !hasMixedTls(convertible)
    );
  });
  const useDefaultGateway =
    migratableBinds.length === 1 && !config.gateways.default;

  for (const bind of config.binds) {
    if (!bindIsMigratable(bind)) {
      remainingBinds.push(bind);
      continue;
    }
    const convertible = bind.listeners.filter(listenerIsMigratable);
    if (
      !convertible.length ||
      convertible.length !== bind.listeners.length ||
      !hasOneRouteKind(convertible) ||
      hasMixedTls(convertible)
    ) {
      remainingBinds.push(bind);
      continue;
    }

    const gatewayName = useDefaultGateway
      ? "default"
      : uniqueGatewayName(config.gateways, `port-${bind.port}`);
    const listenerlessGateway = convertible.length === 1 && useDefaultGateway;
    const gateway: TrafficGateway = listenerlessGateway
      ? migratedSingleListenerGateway(convertible[0], bind.port)
      : {
          port: bind.port,
          listeners: convertible.map((listener, listenerIndex) =>
            migratedGatewayListener(listener, listenerIndex),
          ),
        };
    config.gateways[gatewayName] = cleanGateway(gateway);

    const migratedRoutes = convertible.flatMap((listener, listenerIndex) =>
      (listener.routes ?? []).map((route) =>
        migratedRoute(
          route,
          convertedRouteGatewayRef(
            gatewayName,
            listener,
            listenerIndex,
            listenerlessGateway,
            useDefaultGateway,
          ),
        ),
      ),
    );
    if (migratedRoutes.length) {
      config.routes = [...(config.routes ?? []), ...migratedRoutes];
    }
    const migratedTcpRoutes = convertible.flatMap((listener, listenerIndex) =>
      (listener.tcpRoutes ?? []).map((route) =>
        migratedTcpRoute(
          route,
          convertedRouteGatewayRef(
            gatewayName,
            listener,
            listenerIndex,
            listenerlessGateway,
            useDefaultGateway,
          ),
        ),
      ),
    );
    if (migratedTcpRoutes.length) {
      config.tcpRoutes = [...(config.tcpRoutes ?? []), ...migratedTcpRoutes];
    }

    convertedListeners += convertible.length;
  }

  if (remainingBinds.length) config.binds = remainingBinds;
  else delete config.binds;
  if (config.routes?.length === 0) delete config.routes;
  if (config.tcpRoutes?.length === 0) delete config.tcpRoutes;
  return convertedListeners;
}

function bindIsMigratable(bind: TrafficBind) {
  return (
    Boolean(bind.port) &&
    (!bind.mode || bind.mode === "standard") &&
    (!bind.tunnelProtocol || bind.tunnelProtocol === "direct") &&
    Array.isArray(bind.listeners)
  );
}

function listenerIsMigratable(listener: TrafficListener) {
  const protocol = listener.protocol ?? "HTTP";
  if (protocol === "HTTP" || protocol === "HTTPS") {
    return (
      (protocol === "HTTP" || Boolean(listener.tls)) &&
      Array.isArray(listener.routes)
    );
  }
  return (
    (protocol === "TCP" || protocol === "TLS") &&
    Array.isArray(listener.tcpRoutes)
  );
}

function hasOneRouteKind(listeners: TrafficListener[]) {
  const first = legacyListenerRouteKind(listeners[0]);
  return listeners.every(
    (listener) => legacyListenerRouteKind(listener) === first,
  );
}

function hasMixedTls(listeners: TrafficListener[]) {
  const first = legacyListenerUsesTls(listeners[0]);
  return listeners.some(
    (listener) => legacyListenerUsesTls(listener) !== first,
  );
}

function legacyListenerRouteKind(listener: TrafficListener) {
  return listener.protocol === "TCP" || listener.protocol === "TLS"
    ? "tcp"
    : "http";
}

function legacyListenerUsesTls(listener: TrafficListener | undefined) {
  return listener?.protocol === "HTTPS" || listener?.protocol === "TLS";
}

function migratedGatewayListener(
  listener: TrafficListener,
  listenerIndex: number,
): TrafficGatewayListener {
  const {
    routes: _routes,
    tcpRoutes: _tcpRoutes,
    protocol,
    ...rest
  } = listener;
  const policies =
    rest.policies && typeof rest.policies === "object" ? rest.policies : {};
  const {
    policies: _policies,
    namespace: _namespace,
    tls,
    ...listenerFields
  } = rest;
  return cleanGatewayListener({
    ...listenerFields,
    protocol: gatewayProtocolFromLegacyListener(protocol),
    tls: protocol === "HTTPS" || protocol === "TLS" ? tls : null,
    ...policies,
    name: migratedListenerName(listener, listenerIndex),
  } as TrafficGatewayListener);
}

function migratedSingleListenerGateway(
  listener: TrafficListener,
  port: number | null | undefined,
): TrafficGateway {
  const {
    routes: _routes,
    tcpRoutes: _tcpRoutes,
    protocol,
    name: _name,
    namespace: _namespace,
    hostname: _hostname,
    ...rest
  } = listener;
  const policies =
    rest.policies && typeof rest.policies === "object" ? rest.policies : {};
  const { policies: _policies, tls, ...gatewayFields } = rest;
  return cleanGateway({
    ...gatewayFields,
    port,
    protocol: gatewayProtocolFromLegacyListener(protocol),
    tls: protocol === "HTTPS" || protocol === "TLS" ? tls : null,
    ...policies,
  } as TrafficGateway);
}

function migratedRoute(
  route: TrafficRoute,
  gatewayRef: string | undefined,
): LocalAttachedRoute {
  const migrated = {
    ...structuredClone(route),
  } as LocalAttachedRoute;
  if (gatewayRef) migrated.gateways = gatewayRef;
  return migrated;
}

function migratedTcpRoute(
  route: TrafficTcpRoute,
  gatewayRef: string | undefined,
): LocalAttachedTCPRoute {
  const migrated = {
    ...structuredClone(route),
  } as LocalAttachedTCPRoute;
  if (gatewayRef) migrated.gateways = gatewayRef;
  return migrated;
}

function gatewayProtocolFromLegacyListener(
  protocol: TrafficListener["protocol"] | undefined,
) {
  return protocol === "TCP" || protocol === "TLS" ? protocol : null;
}

function migratedListenerName(
  listener: TrafficListener,
  listenerIndex: number,
) {
  return listener.name?.trim() || `listener${listenerIndex}`;
}

function convertedRouteGatewayRef(
  gatewayName: string,
  listener: TrafficListener,
  listenerIndex: number,
  listenerlessGateway: boolean,
  implicitDefaultGateway: boolean,
) {
  if (implicitDefaultGateway) return undefined;
  if (listenerlessGateway) return gatewayName;
  return `${gatewayName}/${migratedListenerName(listener, listenerIndex)}`;
}

function uniqueGatewayName(
  gateways: NonNullable<GatewayConfig["gateways"]>,
  base: string,
) {
  if (!gateways[base]) return base;
  for (let index = 2; ; index += 1) {
    const candidate = `${base}-${index}`;
    if (!gateways[candidate]) return candidate;
  }
}

function gatewayPortLabel(gateway: TrafficGateway) {
  return gateway.port?.toString() ?? "Unset";
}

function gatewayHasTls(gateway: TrafficGateway) {
  if (gateway.listeners?.length) {
    return gateway.listeners.some((listener) => {
      const protocol = effectiveGatewayProtocol(listener);
      return protocol === "HTTPS" || protocol === "TLS";
    });
  }
  const protocol = effectiveGatewayProtocol(gateway);
  return protocol === "HTTPS" || protocol === "TLS";
}

function gatewayProtocolLabel(
  gateway: TrafficGateway | TrafficGatewayListener,
) {
  return effectiveGatewayProtocol(gateway);
}

function gatewayPolicyCount(gateway: TrafficGateway) {
  return Object.keys(gatewayPolicies(gateway)).length;
}

function gatewayListenerName(listener: TrafficGatewayListener, index: number) {
  return listener.name || `listener${index + 1}`;
}

function gatewayListenerPolicyCount(listener: TrafficGatewayListener) {
  return Object.keys(gatewayListenerPolicies(listener)).length;
}

function gatewayPolicies(gateway: TrafficGateway) {
  const {
    listeners: _listeners,
    port: _port,
    tls: _tls,
    ...policies
  } = gateway;
  return policies as Record<string, unknown>;
}

function withoutGatewayPolicies(gateway: TrafficGateway): TrafficGateway {
  return {
    listeners: gateway.listeners,
    port: gateway.port,
    tls: gateway.tls,
  };
}

function gatewayListenerPolicies(listener: TrafficGatewayListener) {
  const { name: _name, hostname: _hostname, tls: _tls, ...policies } = listener;
  return policies as Record<string, unknown>;
}

function withoutGatewayListenerPolicies(
  listener: TrafficGatewayListener,
): TrafficGatewayListener {
  return {
    name: listener.name,
    hostname: listener.hostname,
    tls: listener.tls,
  };
}
