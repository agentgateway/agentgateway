import { Link } from "@tanstack/react-router";
import {
  Network,
  Pencil,
  Plus,
  Route as RouteIcon,
  Save,
  Trash2,
} from "lucide-react";
import { useMemo, useState } from "react";
import { EnumSelector } from "../components/EnumSelector";
import {
  Drawer,
  Dropdown,
  EmptyState,
  Field,
  FieldGroup,
  PageHeader,
  Panel,
  StatusBanner,
  Tooltip,
  YamlBlock,
} from "../components/Primitives";
import { useStickyQueryParam } from "../drawerRouteState";
import { useConfigDumpMode, useGatewayConfig, useUpdateConfig } from "../hooks";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import {
  listenerContexts,
  listenerDisplayName,
  trafficStats,
} from "../traffic";
import type { TrafficBind, TrafficListener } from "../types";
import type { LocalTLSServerConfig } from "../gateway-config";
import {
  ReadonlyModeBanner,
  TrafficDumpListenersView,
} from "./traffic/TrafficConfigDumpPanel";
import { TrafficPolicySection } from "./traffic/TrafficPolicySection";

const protocols = ["HTTP", "HTTPS", "TCP", "TLS", "HBONE"] as const;

export function TrafficListenersPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return (
      <div className="page-stack">
        <PageHeader
          title="Traffic Listeners"
          description="Configure bind ports and listeners for generic HTTP and TCP traffic."
        />
        <Panel>
          <StatusBanner
            state="loading"
            title="Detecting traffic configuration mode"
          />
        </Panel>
      </div>
    );
  }
  if (mode.data?.mode === "dump") {
    return (
      <div className="page-stack">
        <PageHeader
          title="Traffic Listeners"
          description="Read-only listener inventory from the active gateway dump."
        />
        <ReadonlyModeBanner />
        <TrafficDumpListenersView dump={mode.data.dump} />
      </div>
    );
  }
  return <TrafficListenersEditorPage />;
}

function TrafficListenersEditorPage() {
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const listeners = useMemo(() => listenerContexts(config.data), [config.data]);
  const stats = trafficStats(config.data);
  const [bindEditor, setBindEditor] = useState<TrafficBind | null>(null);
  const [listenerEditor, setListenerEditor] = useState<{
    bindIndex: number;
    listenerIndex?: number;
    listener: TrafficListener;
  } | null>(null);
  const [trafficDrawer, setTrafficDrawer] = useStickyQueryParam("drawer");
  const linkedBind = linkedBindEditor(trafficDrawer, config.data?.binds ?? []);
  const linkedListener = linkedListenerEditor(
    trafficDrawer,
    config.data?.binds ?? [],
  );
  const activeBindEditor = bindEditor ?? linkedBind;
  const activeListenerEditor = listenerEditor ?? linkedListener;

  function openBindEditor(bind: TrafficBind | null, bindIndex?: number) {
    setListenerEditor(null);
    setBindEditor(null);
    setTrafficDrawer(bind ? `bind:${bindIndex ?? 0}` : "bind:new");
  }

  function openListenerEditor(
    bindIndex: number,
    listener: TrafficListener | null,
    listenerIndex?: number,
  ) {
    setBindEditor(null);
    setListenerEditor(null);
    setTrafficDrawer(
      listener && typeof listenerIndex === "number"
        ? `listener:${bindIndex}:${listenerIndex}`
        : `listener:new:${bindIndex}`,
    );
  }

  function closeTrafficDrawer() {
    setBindEditor(null);
    setListenerEditor(null);
    setTrafficDrawer(null, "replace");
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Traffic Listeners"
        description="Configure bind ports and listeners for generic HTTP and TCP traffic."
        actions={
          <div className="button-row">
            <button
              className="button"
              type="button"
              onClick={() => openBindEditor(null)}
            >
              <Plus size={16} />
              Add bind
            </button>
            <button
              className="button primary"
              type="button"
              disabled={!config.data?.binds?.length}
              onClick={() => openListenerEditor(0, null)}
            >
              <Plus size={16} />
              Add listener
            </button>
          </div>
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
      {stats.invalidListeners ? (
        <StatusBanner
          state="warn"
          title={`${stats.invalidListeners} listener${stats.invalidListeners === 1 ? "" : "s"} mix HTTP and TCP routes`}
        >
          Edit those listeners through raw YAML or split the routes across
          separate listeners.
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading traffic listeners" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {config.error.message}
          </StatusBanner>
        ) : !config.data?.binds?.length ? (
          <EmptyState
            title="No traffic binds configured"
            description="Add a bind port before attaching listeners and routes."
            action={
              <button
                className="button primary"
                type="button"
                onClick={() => openBindEditor(null)}
              >
                <Network size={16} />
                Add bind
              </button>
            }
          />
        ) : (
          <div className="traffic-bind-list">
            {config.data.binds.map((bind, bindIndex) => {
              const bindListeners = listeners.filter(
                (item) => item.bindIndex === bindIndex,
              );
              const backendCount = bindListeners.reduce(
                (total, item) => total + listenerBackendCount(item.listener),
                0,
              );
              return (
                <section
                  className="traffic-bind"
                  key={`${bind.port}-${bindIndex}`}
                >
                  <div className="traffic-bind-header">
                    <div>
                      <h3>Port {bind.port}</h3>
                      <p>
                        {bindListeners.length} listeners ·{" "}
                        {listenerRouteCount(bind)} routes · {backendCount}{" "}
                        backends
                      </p>
                    </div>
                    <div className="button-row">
                      <span className="badge">
                        {bind.tunnelProtocol ?? "direct"}
                      </span>
                      <Tooltip content="Add listener">
                        <button
                          className="icon-button"
                          type="button"
                          aria-label="Add listener"
                          onClick={() => openListenerEditor(bindIndex, null)}
                        >
                          <Plus size={16} />
                        </button>
                      </Tooltip>
                      <Tooltip content="Edit bind">
                        <button
                          className="icon-button"
                          type="button"
                          aria-label="Edit bind"
                          onClick={() => openBindEditor(bind, bindIndex)}
                        >
                          <Pencil size={16} />
                        </button>
                      </Tooltip>
                      <Tooltip content="Delete bind">
                        <button
                          className="icon-button danger"
                          type="button"
                          aria-label="Delete bind"
                          onClick={() =>
                            update.mutate((next) => {
                              next.binds = (next.binds ?? []).filter(
                                (_, index) => index !== bindIndex,
                              );
                            })
                          }
                        >
                          <Trash2 size={16} />
                        </button>
                      </Tooltip>
                    </div>
                  </div>
                  {bind.listeners.length ? (
                    <div className="table-wrap">
                      <table>
                        <thead>
                          <tr>
                            <th>Name</th>
                            <th>Hostname</th>
                            <th>Protocol</th>
                            <th>Routes</th>
                            <th>Backends</th>
                            <th />
                          </tr>
                        </thead>
                        <tbody>
                          {bind.listeners.map((listener, listenerIndex) => (
                            <tr key={`${listener.name}-${listenerIndex}`}>
                              <td className="strong">
                                {listenerDisplayName(listener, listenerIndex)}
                              </td>
                              <td>{listener.hostname || "*"}</td>
                              <td>
                                <span className="badge">
                                  {listener.protocol ?? "HTTP"}
                                </span>
                              </td>
                              <td>
                                {(listener.routes?.length ?? 0) +
                                  (listener.tcpRoutes?.length ?? 0)}
                              </td>
                              <td>{listenerBackendCount(listener)}</td>
                              <td className="row-actions">
                                <Tooltip content="Add route">
                                  <Link
                                    className="icon-button"
                                    aria-label="Add route"
                                    to="/traffic/routes"
                                    search={{
                                      listener: `${bindIndex}:${listenerIndex}`,
                                    }}
                                  >
                                    <RouteIcon size={16} />
                                  </Link>
                                </Tooltip>
                                <Tooltip content="Edit listener">
                                  <button
                                    className="icon-button"
                                    type="button"
                                    aria-label="Edit listener"
                                    onClick={() =>
                                      openListenerEditor(
                                        bindIndex,
                                        listener,
                                        listenerIndex,
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
                                        const target = next.binds?.[bindIndex];
                                        if (target)
                                          target.listeners =
                                            target.listeners.filter(
                                              (_, index) =>
                                                index !== listenerIndex,
                                            );
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
                  ) : (
                    <EmptyState
                      title="No listeners on this bind"
                      description="Add a listener to start matching traffic on this port."
                    />
                  )}
                </section>
              );
            })}
          </div>
        )}
      </Panel>

      {activeBindEditor ? (
        <BindEditor
          key={`${trafficDrawer ?? "bind-local"}-${activeBindEditor.port}`}
          bind={activeBindEditor}
          help={help}
          saving={update.isPending}
          onCancel={closeTrafficDrawer}
          onSave={(bind) =>
            update.mutate(
              (next) => {
                if (!Array.isArray(next.binds)) next.binds = [];
                const index = next.binds.findIndex(
                  (item) => item.port === activeBindEditor.port,
                );
                if (index >= 0) next.binds[index] = bind;
                else next.binds.push(bind);
              },
              { onSuccess: closeTrafficDrawer },
            )
          }
        />
      ) : null}

      {activeListenerEditor ? (
        <ListenerEditor
          binds={config.data?.binds ?? []}
          key={trafficDrawer ?? "listener-local"}
          editing={activeListenerEditor}
          help={help}
          saving={update.isPending}
          onCancel={closeTrafficDrawer}
          onSave={(bindIndex, listener, listenerIndex) =>
            update.mutate(
              (next) => {
                const bind = next.binds?.[bindIndex];
                if (!bind) return;
                if (typeof listenerIndex === "number")
                  bind.listeners[listenerIndex] = listener;
                else bind.listeners.push(listener);
              },
              { onSuccess: closeTrafficDrawer },
            )
          }
        />
      ) : null}
    </div>
  );
}

function BindEditor(props: {
  bind: TrafficBind;
  help: SchemaHelp;
  saving: boolean;
  onCancel: () => void;
  onSave: (bind: TrafficBind) => void;
}) {
  const [port, setPort] = useState(String(props.bind.port));
  const [error, setError] = useState<string | null>(null);
  const preview: TrafficBind = {
    ...props.bind,
    port: Number(port),
  };

  function save() {
    const parsed = Number(port);
    if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
      setError("Port must be between 1 and 65535.");
      return;
    }
    props.onSave({ ...preview, port: parsed });
  }

  return (
    <Drawer
      title="Bind port"
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            disabled={props.saving}
            onClick={save}
          >
            <Save size={16} />
            Save bind
          </button>
        </div>
      }
    >
      {error ? <StatusBanner state="bad" title={error} /> : null}
      <div className="form-grid">
        <Field
          label="Port"
          tooltip={props.help.field<TrafficBind>("LocalBind", "port")}
        >
          <input
            value={port}
            onChange={(event) => setPort(event.target.value)}
          />
        </Field>
      </div>
      <details open>
        <summary>Resulting YAML</summary>
        <YamlBlock value={preview} />
      </details>
    </Drawer>
  );
}

function ListenerEditor(props: {
  binds: TrafficBind[];
  editing: {
    bindIndex: number;
    listenerIndex?: number;
    listener: TrafficListener;
  };
  help: SchemaHelp;
  saving: boolean;
  onCancel: () => void;
  onSave: (
    bindIndex: number,
    listener: TrafficListener,
    listenerIndex?: number,
  ) => void;
}) {
  const [bindIndex, setBindIndex] = useState(String(props.editing.bindIndex));
  const [listener, setListener] = useState<TrafficListener>(
    props.editing.listener,
  );
  const [cert, setCert] = useState(listener.tls?.cert ?? "");
  const [key, setKey] = useState(listener.tls?.key ?? "");
  const protocol = listener.protocol ?? "HTTP";
  const supportsTcp = protocol === "TCP" || protocol === "TLS";
  const preview: TrafficListener = {
    ...listener,
    ...(supportsTcp
      ? { routes: undefined, tcpRoutes: listener.tcpRoutes ?? [] }
      : { routes: listener.routes ?? [], tcpRoutes: undefined }),
    tls:
      cert.trim() || key.trim()
        ? { ...(listener.tls ?? {}), cert: cert.trim(), key: key.trim() }
        : null,
  };

  return (
    <Drawer
      title={
        typeof props.editing.listenerIndex === "number"
          ? "Edit listener"
          : "Add listener"
      }
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>
            Cancel
          </button>
          <button
            className="button primary"
            type="button"
            disabled={props.saving}
            onClick={() =>
              props.onSave(
                Number(bindIndex),
                cleanListener(preview),
                props.editing.listenerIndex,
              )
            }
          >
            <Save size={16} />
            Save listener
          </button>
        </div>
      }
    >
      <div className="form-grid">
        {typeof props.editing.listenerIndex !== "number" ? (
          <FieldGroup
            label="Bind"
            tooltip="Bind port this listener is attached to."
          >
            <Dropdown
              ariaLabel="Bind"
              value={bindIndex}
              options={props.binds.map((bind, index) => ({
                value: String(index),
                label: `Port ${bind.port}`,
              }))}
              onChange={setBindIndex}
            />
          </FieldGroup>
        ) : null}
        <Field
          label="Name"
          tooltip={props.help.field<TrafficListener>("LocalListener", "name")}
        >
          <input
            value={listener.name ?? ""}
            onChange={(event) =>
              setListener({ ...listener, name: event.target.value })
            }
            placeholder="public-http"
          />
        </Field>
        <Field
          label="Hostname"
          tooltip={props.help.field<TrafficListener>(
            "LocalListener",
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
        <FieldGroup
          label="Protocol"
          tooltip={props.help.field<TrafficListener>(
            "LocalListener",
            "protocol",
          )}
        >
          <EnumSelector
            ariaLabel="Protocol"
            value={protocol}
            options={protocols.map((value) => ({ value, label: value }))}
            schema={props.help.node([
              "$defs",
              "LocalListener",
              "properties",
              "protocol",
            ])}
            onChange={(value) =>
              setListener(makeProtocolListener(listener, value))
            }
          />
        </FieldGroup>
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
        policies={
          listener.policies as Record<string, unknown> | null | undefined
        }
        onChange={(policies) => setListener({ ...listener, policies })}
      />
      <details open>
        <summary>Resulting YAML</summary>
        <YamlBlock value={cleanListener(preview)} />
      </details>
    </Drawer>
  );
}

function makeListener(protocol: TrafficListener["protocol"]): TrafficListener {
  return makeProtocolListener({ name: "", hostname: null }, protocol);
}

function makeProtocolListener(
  listener: TrafficListener,
  protocol: TrafficListener["protocol"],
): TrafficListener {
  const supportsTcp = protocol === "TCP" || protocol === "TLS";
  return {
    ...listener,
    protocol,
    routes: supportsTcp ? undefined : (listener.routes ?? []),
    tcpRoutes: supportsTcp ? (listener.tcpRoutes ?? []) : undefined,
  };
}

function cleanListener(listener: TrafficListener): TrafficListener {
  const next = { ...listener };
  if (!next.name) delete next.name;
  if (!next.hostname) delete next.hostname;
  if (!next.tls) delete next.tls;
  if (!next.routes) delete next.routes;
  if (!next.tcpRoutes) delete next.tcpRoutes;
  if (!next.policies) delete next.policies;
  return next;
}

function listenerRouteCount(bind: TrafficBind) {
  return bind.listeners.reduce(
    (total, listener) =>
      total +
      (listener.routes?.length ?? 0) +
      (listener.tcpRoutes?.length ?? 0),
    0,
  );
}

function listenerBackendCount(listener: TrafficListener) {
  const http =
    listener.routes?.reduce(
      (total, route) => total + (route.backends?.length ?? 0),
      0,
    ) ?? 0;
  const tcp =
    listener.tcpRoutes?.reduce(
      (total, route) => total + (route.backends?.length ?? 0),
      0,
    ) ?? 0;
  return http + tcp;
}

function linkedBindEditor(value: string | null, binds: TrafficBind[]) {
  if (!value?.startsWith("bind:")) return null;
  if (value === "bind:new") return { port: 8080, listeners: [] } as TrafficBind;
  const bindIndex = Number(value.slice("bind:".length));
  const bind = Number.isInteger(bindIndex) ? binds[bindIndex] : undefined;
  return bind ? structuredClone(bind) : null;
}

function linkedListenerEditor(value: string | null, binds: TrafficBind[]) {
  if (!value?.startsWith("listener:")) return null;
  const parts = value.split(":");
  if (parts[1] === "new") {
    const bindIndex = Number(parts[2] ?? 0);
    return Number.isInteger(bindIndex) && binds[bindIndex]
      ? { bindIndex, listener: makeListener("HTTP") }
      : null;
  }
  const bindIndex = Number(parts[1]);
  const listenerIndex = Number(parts[2]);
  const listener =
    Number.isInteger(bindIndex) && Number.isInteger(listenerIndex)
      ? binds[bindIndex]?.listeners?.[listenerIndex]
      : undefined;
  return listener
    ? { bindIndex, listenerIndex, listener: structuredClone(listener) }
    : null;
}
