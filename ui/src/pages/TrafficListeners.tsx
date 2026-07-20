import { tr } from "../i18n";
import { Link } from "@tanstack/react-router";
import {
  Network,
  Pencil,
  Plus,
  Route as RouteIcon,
  Trash2,
} from "lucide-react";
import { useMemo, useState } from "react";
import { EnumSelector } from "../components/EnumSelector";
import {
  ConfirmDialog,
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
import { ConfigDiffSaveActions } from "../components/ConfigDiffDrawer";
import { useStickyQueryParam } from "../drawerRouteState";
import { useConfigDumpMode, useGatewayConfig, useUpdateConfig } from "../hooks";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import {
  listenerContexts,
  listenerDisplayName,
  trafficStats,
} from "../traffic";
import type { GatewayConfig, TrafficBind, TrafficListener } from "../types";
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
          title={tr("copy.trafficListeners")}
          description={tr(
            "copy.configureBindPortsAndListenersForGenericHttpAndTcpTraffic",
          )}
        />
        <Panel>
          <StatusBanner
            state="loading"
            title={tr("copy.detectingTrafficConfigurationMode")}
          />
        </Panel>
      </div>
    );
  }
  if (mode.data?.mode === "dump") {
    return (
      <div className="page-stack">
        <PageHeader
          title={tr("copy.trafficListeners")}
          description={tr(
            "copy.readOnlyListenerInventoryFromTheActiveGatewayDump",
          )}
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
  const [deleting, setDeleting] = useState<
    | {
        kind: "bind";
        bindIndex: number;
        label: string;
        listenerCount: number;
      }
    | {
        kind: "listener";
        bindIndex: number;
        listenerIndex: number;
        label: string;
      }
    | null
  >(null);
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
        title={tr("copy.trafficListeners")}
        description={tr(
          "copy.configureBindPortsAndListenersForGenericHttpAndTcpTraffic",
        )}
        actions={
          <div className="button-row">
            <button
              className="button"
              type="button"
              onClick={() => openBindEditor(null)}
            >
              <Plus size={16} />
              {tr("copy.addBind")}
            </button>
            <button
              className="button primary"
              type="button"
              disabled={!config.data?.binds?.length}
              onClick={() => openListenerEditor(0, null)}
            >
              <Plus size={16} />
              {tr("copy.addListener")}
            </button>
          </div>
        }
      />

      {update.isError ? (
        <StatusBanner state="bad" title={tr("copy.saveFailed")}>
          {update.error.message}
        </StatusBanner>
      ) : null}
      {update.isSuccess ? (
        <StatusBanner state="ok" title={tr("copy.configurationSaved")} />
      ) : null}
      {stats.invalidListeners ? (
        <StatusBanner
          state="warn"
          title={tr("copy.valueListenerValueMixHttpAndTcpRoutes")}
        >
          {tr(
            "copy.editThoseListenersThroughRawYamlOrSplitTheRoutesAcrossSeparateListeners",
          )}
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner
            state="loading"
            title={tr("copy.loadingTrafficListeners")}
          />
        ) : config.isError ? (
          <StatusBanner
            state="bad"
            title={tr("copy.configurationApiUnavailable")}
          >
            {config.error.message}
          </StatusBanner>
        ) : !config.data?.binds?.length ? (
          <EmptyState
            title={tr("copy.noLegacyBindsConfigured")}
            description={tr(
              "copy.useTrafficGatewaysForNewHttpRoutingConfiguration",
            )}
            action={
              <Link className="button primary" to="/traffic/gateways">
                <Network size={16} />
                {tr("copy.manageGateways")}
              </Link>
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
                      <h3>
                        {tr("copy.port")}
                        {bind.port}
                      </h3>
                      <p>
                        {bindListeners.length}
                        {tr("copy.listeners_1fzojr3")}{" "}
                        {listenerRouteCount(bind)}
                        {tr("copy.routes_4p3286")}
                        {backendCount} {tr("copy.backends")}
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
                          aria-label={tr("copy.addListener")}
                          onClick={() => openListenerEditor(bindIndex, null)}
                        >
                          <Plus size={16} />
                        </button>
                      </Tooltip>
                      <Tooltip content="Edit bind">
                        <button
                          className="icon-button"
                          type="button"
                          aria-label={tr("copy.editBind")}
                          onClick={() => openBindEditor(bind, bindIndex)}
                        >
                          <Pencil size={16} />
                        </button>
                      </Tooltip>
                      <Tooltip content="Delete bind">
                        <button
                          className="icon-button danger"
                          type="button"
                          aria-label={tr("copy.deleteBind")}
                          disabled={update.isPending}
                          onClick={() =>
                            setDeleting({
                              kind: "bind",
                              bindIndex,
                              label: tr("copy.portValue", [bind.port]),
                              listenerCount: bind.listeners.length,
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
                            <th>{tr("copy.name")}</th>
                            <th>{tr("copy.hostname")}</th>
                            <th>{tr("copy.protocol")}</th>
                            <th>{tr("copy.routes_14u6307")}</th>
                            <th>{tr("copy.backends_i9thuc")}</th>
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
                                    aria-label={tr("copy.addRoute")}
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
                                    aria-label={tr("copy.editListener")}
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
                                    aria-label={tr("copy.deleteListener")}
                                    disabled={update.isPending}
                                    onClick={() =>
                                      setDeleting({
                                        kind: "listener",
                                        bindIndex,
                                        listenerIndex,
                                        label: listenerDisplayName(
                                          listener,
                                          listenerIndex,
                                        ),
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
                      title={tr("copy.noListenersOnThisBind")}
                      description={tr(
                        "copy.addAListenerToStartMatchingTrafficOnThisPort",
                      )}
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
          config={config.data}
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
          config={config.data}
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
      {deleting ? (
        <ConfirmDialog
          title={tr("copy.deleteValue_pkbukw")}
          destructive
          confirmLabel={tr("copy.deleteValue")}
          confirmDisabled={update.isPending}
          onCancel={() => setDeleting(null)}
          onConfirm={() =>
            update.mutate(
              (next) => {
                if (deleting.kind === "bind") {
                  next.binds = (next.binds ?? []).filter(
                    (_, index) => index !== deleting.bindIndex,
                  );
                  return;
                }
                const bind = next.binds?.[deleting.bindIndex];
                if (bind)
                  bind.listeners = bind.listeners.filter(
                    (_, index) => index !== deleting.listenerIndex,
                  );
              },
              { onSuccess: () => setDeleting(null) },
            )
          }
        >
          <p>
            {tr("copy.delete")}
            <strong>{deleting.label}</strong>?
            {deleting.kind === "bind" && deleting.listenerCount
              ? ` This also removes ${deleting.listenerCount} listener${deleting.listenerCount === 1 ? "" : "s"} and their routes.`
              : " Traffic using it will no longer be served."}
          </p>
        </ConfirmDialog>
      ) : null}
    </div>
  );
}

function BindEditor(props: {
  bind: TrafficBind;
  config?: GatewayConfig;
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
      setError(tr("copy.portMustBeBetween1And65535"));
      return;
    }
    props.onSave({ ...preview, port: parsed });
  }

  return (
    <Drawer
      title={tr("copy.bindPort")}
      onClose={props.onCancel}
      dirty={port !== String(props.bind.port)}
      saving={props.saving}
      footer={(requestClose) => (
        <ConfigDiffSaveActions
          config={props.config}
          diffTitle="Bind config diff"
          saveLabel="Save bind"
          saving={props.saving}
          onCancel={requestClose}
          onSave={save}
          beforeDiff={() => {
            const parsed = Number(port);
            if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
              setError(tr("copy.portMustBeBetween1And65535"));
              return false;
            }
            return true;
          }}
          applyDiff={(next) => {
            if (!Array.isArray(next.binds)) next.binds = [];
            const index = next.binds.findIndex(
              (item) => item.port === props.bind.port,
            );
            const parsed = Number(port);
            const bind = { ...preview, port: parsed };
            if (index >= 0) next.binds[index] = bind;
            else next.binds.push(bind);
          }}
        />
      )}
    >
      {error ? <StatusBanner state="bad" title={error} /> : null}
      <div className="form-grid">
        <Field
          label={tr("copy.port")}
          tooltip={props.help.field<TrafficBind>("LocalBind", "port")}
        >
          <input
            value={port}
            onChange={(event) => setPort(event.target.value)}
          />
        </Field>
      </div>
      <details open>
        <summary>{tr("copy.resultingYaml")}</summary>
        <YamlBlock value={preview} />
      </details>
    </Drawer>
  );
}

function ListenerEditor(props: {
  binds: TrafficBind[];
  config?: GatewayConfig;
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
  const draft = JSON.stringify({ bindIndex, listener, cert, key });
  const [initialDraft] = useState(() => draft);
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
      dirty={draft !== initialDraft}
      saving={props.saving}
      footer={(requestClose) => (
        <ConfigDiffSaveActions
          config={props.config}
          diffTitle="Listener config diff"
          saveLabel="Save listener"
          saving={props.saving}
          onCancel={requestClose}
          onSave={() =>
            props.onSave(
              Number(bindIndex),
              cleanListener(preview),
              props.editing.listenerIndex,
            )
          }
          applyDiff={(next) => {
            const bind = next.binds?.[Number(bindIndex)];
            if (!bind) return;
            if (typeof props.editing.listenerIndex === "number") {
              bind.listeners[props.editing.listenerIndex] =
                cleanListener(preview);
            } else {
              bind.listeners.push(cleanListener(preview));
            }
          }}
        />
      )}
    >
      <div className="form-grid">
        {typeof props.editing.listenerIndex !== "number" ? (
          <FieldGroup
            label={tr("copy.bind")}
            tooltip={tr("copy.bindPortThisListenerIsAttachedTo")}
          >
            <Dropdown
              ariaLabel="Bind"
              value={bindIndex}
              options={props.binds.map((bind, index) => ({
                value: String(index),
                label: tr("copy.portValue", [bind.port]),
              }))}
              onChange={setBindIndex}
            />
          </FieldGroup>
        ) : null}
        <Field
          label={tr("copy.name")}
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
          label={tr("copy.hostname")}
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
          label={tr("copy.protocol")}
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
            label={tr("copy.certificate")}
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
            label={tr("copy.key")}
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
        title={tr("copy.listenerPolicies")}
        schemaRoot="LocalGatewayPolicy"
        policies={
          listener.policies as Record<string, unknown> | null | undefined
        }
        onChange={(policies) => setListener({ ...listener, policies })}
      />
      <details open>
        <summary>{tr("copy.resultingYaml")}</summary>
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
