import { Network, Pencil, Plus, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import {
  Drawer,
  EmptyState,
  Field,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import { ConfigDiffSaveActions } from "../components/ConfigDiffDrawer";
import { useStickyQueryParam } from "../drawerRouteState";
import { useGatewayConfig, useUpdateConfig } from "../hooks";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import type {
  GatewayConfig,
  TrafficGateway,
  TrafficGatewayListener,
} from "../types";
import type { LocalTLSServerConfig } from "../gateway-config";
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

export function TrafficGatewaysPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const [drawer, setDrawer] = useStickyQueryParam("gateway");
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

      {update.isError ? (
        <StatusBanner state="bad" title="Save failed">
          {update.error.message}
        </StatusBanner>
      ) : null}
      {update.isSuccess ? (
        <StatusBanner state="ok" title="Configuration saved" />
      ) : null}
      {showLegacyBindsWarning ? (
        <StatusBanner
          state="warn"
          title="Detected legacy binds config"
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
            {config.error.message}
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
                          onClick={() => setDrawer(`listener:new:${name}`)}
                        >
                          <Plus size={16} />
                        </button>
                      </Tooltip>
                    ) : null}
                    <Tooltip content="Edit gateway">
                      <button
                        className="icon-button"
                        type="button"
                        aria-label="Edit gateway"
                        onClick={() => setDrawer(name)}
                      >
                        <Pencil size={16} />
                      </button>
                    </Tooltip>
                    <Tooltip content="Delete gateway">
                      <button
                        className="icon-button danger"
                        type="button"
                        aria-label="Delete gateway"
                        onClick={() =>
                          update.mutate((next) => {
                            if (!next.gateways) return;
                            delete next.gateways[name];
                            if (Object.keys(next.gateways).length === 0) {
                              delete next.gateways;
                            }
                          })
                        }
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
                            <td>
                              <span
                                className={listener.tls ? "badge ok" : "badge"}
                              >
                                {listener.tls ? "TLS" : "Plain"}
                              </span>
                            </td>
                            <td>{gatewayListenerPolicyCount(listener)}</td>
                            <td className="row-actions">
                              <Tooltip content="Edit listener">
                                <button
                                  className="icon-button"
                                  type="button"
                                  aria-label="Edit listener"
                                  onClick={() =>
                                    setDrawer(
                                      `listener:edit:${name}:${listenerIndex}`,
                                    )
                                  }
                                >
                                  <Pencil size={16} />
                                </button>
                              </Tooltip>
                              <Tooltip content="Delete listener">
                                <button
                                  className="icon-button danger"
                                  type="button"
                                  aria-label="Delete listener"
                                  onClick={() =>
                                    update.mutate((next) => {
                                      const target = next.gateways?.[name];
                                      if (!target?.listeners) return;
                                      target.listeners =
                                        target.listeners.filter(
                                          (_, index) => index !== listenerIndex,
                                        );
                                      if (target.listeners.length === 0) {
                                        delete target.listeners;
                                      }
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
          saving={update.isPending}
          onCancel={closeDrawer}
          onSave={(draft) =>
            update.mutate(
              (next) => {
                next.gateways ??= {};
                if (draft.previousName && draft.previousName !== draft.name) {
                  delete next.gateways[draft.previousName];
                }
                next.gateways[draft.name] = cleanGateway(draft.gateway);
              },
              { onSuccess: closeDrawer },
            )
          }
        />
      ) : null}

      {activeListener ? (
        <GatewayListenerEditor
          key={drawer ?? "listener-local"}
          editing={activeListener}
          config={config.data}
          help={help}
          saving={update.isPending}
          onCancel={closeDrawer}
          onSave={(gatewayName, listener, listenerIndex) =>
            update.mutate(
              (next) => {
                const gateway = next.gateways?.[gatewayName];
                if (!gateway) return;
                gateway.listeners ??= [];
                if (typeof listenerIndex === "number") {
                  gateway.listeners[listenerIndex] = listener;
                } else {
                  gateway.listeners.push(listener);
                }
              },
              { onSuccess: closeDrawer },
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
  onCancel: () => void;
  onSave: (draft: GatewayEditorState) => void;
}) {
  const [name, setName] = useState(props.initial.name);
  const [gateway, setGateway] = useState<TrafficGateway>(
    structuredClone(props.initial.gateway),
  );
  const [multipleListeners, setMultipleListeners] = useState(
    Boolean(gateway.listeners?.length),
  );
  const [cert, setCert] = useState(gateway.tls?.cert ?? "");
  const [key, setKey] = useState(gateway.tls?.key ?? "");
  const policyCount = gatewayPolicyCount(gateway);
  const hasTls = Boolean(cert.trim() || key.trim() || gateway.tls);
  const canEnableMultipleListeners = !hasTls && policyCount === 0;
  const preview: TrafficGateway = cleanGateway({
    ...(multipleListeners ? withoutGatewayPolicies(gateway) : gateway),
    listeners: multipleListeners
      ? gateway.listeners?.length
        ? gateway.listeners
        : [{ name: "listener1" }]
      : [],
    tls:
      !multipleListeners && (cert.trim() || key.trim())
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
          diffTitle="Gateway config diff"
          saveLabel="Save gateway"
          saving={props.saving}
          saveDisabled={!name.trim()}
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
      <div className="form-grid">
        <Field
          label="Name"
          tooltip="Features and routes reference this gateway by name."
        >
          <input
            value={name}
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
        <GatewayTLSFields
          cert={cert}
          keyValue={key}
          help={props.help}
          onCertChange={setCert}
          onKeyChange={setKey}
        />
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
  const preview = cleanGatewayListener({
    ...listener,
    tls:
      cert.trim() || key.trim()
        ? { ...(listener.tls ?? {}), cert: cert.trim(), key: key.trim() }
        : null,
  });

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
              value={cert}
              onChange={(event) => setCert(event.target.value)}
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
              value={key}
              onChange={(event) => setKey(event.target.value)}
              placeholder="/etc/certs/tls.key"
            />
          </Field>
        </div>
      </details>
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
  return Object.fromEntries(
    Object.entries(next).filter(
      ([, value]) => value !== null && value !== undefined,
    ),
  ) as TrafficGatewayListener;
}

function gatewayPortLabel(gateway: TrafficGateway) {
  return gateway.port?.toString() ?? "Unset";
}

function gatewayHasTls(gateway: TrafficGateway) {
  if (gateway.listeners?.length) {
    return gateway.listeners.some((listener) => Boolean(listener.tls));
  }
  return Boolean(gateway.tls);
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
