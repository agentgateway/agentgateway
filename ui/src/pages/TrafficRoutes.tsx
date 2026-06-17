import { Link, useLocation } from "@tanstack/react-router";
import { Pencil, Plus, Route as RouteIcon, Save, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import {
  EnumSelector,
  type EnumSelectorOption,
} from "../components/EnumSelector";
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
import { useConfigDumpMode, useGatewayConfig, useUpdateConfig } from "../hooks";
import { useSchemaHelp, type SchemaHelp } from "../schemaHelp";
import {
  backendSummary,
  listenerDisplayName,
  pathSummary,
  routeArray,
  routeContexts,
  routeDisplayName,
  trafficStats,
  type RouteKind,
} from "../traffic";
import type {
  TrafficListener,
  TrafficRoute,
  TrafficRouteBackend,
  TrafficTcpRoute,
  TrafficTcpRouteBackend,
} from "../types";
import type { RouteMatch as GeneratedRouteMatch } from "../gateway-config";
import {
  ReadonlyModeBanner,
  TrafficDumpRoutesView,
} from "./traffic/TrafficConfigDumpPanel";
import { TrafficPolicySection } from "./traffic/TrafficPolicySection";

const pathTypes = ["pathPrefix", "exact", "regex"] as const;
type HttpMatch = NonNullable<TrafficRoute["matches"]>[number];
type HeaderMatch = NonNullable<HttpMatch["headers"]>[number];
type QueryMatch = NonNullable<HttpMatch["query"]>[number];

export function TrafficRoutesPage() {
  const mode = useConfigDumpMode();
  if (mode.isLoading) {
    return (
      <div className="page-stack">
        <PageHeader
          title="Traffic Routes"
          description="Match incoming HTTP and TCP traffic and attach inline backends."
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
          title="Traffic Routes"
          description="Read-only route inventory from the active gateway dump."
        />
        <ReadonlyModeBanner />
        <TrafficDumpRoutesView dump={mode.data.dump} />
      </div>
    );
  }
  return <TrafficRoutesEditorPage />;
}

function TrafficRoutesEditorPage() {
  const location = useLocation();
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const help = useSchemaHelp();
  const routes = useMemo(() => routeContexts(config.data), [config.data]);
  const listeners = useMemo(
    () =>
      (config.data?.binds ?? []).flatMap((bind, bindIndex) =>
        bind.listeners.map((listener, listenerIndex) => ({
          bind,
          bindIndex,
          listener,
          listenerIndex,
        })),
      ),
    [config.data],
  );
  const stats = trafficStats(config.data);
  const [editing, setEditing] = useState<{
    bindIndex: number;
    listenerIndex: number;
    kind: RouteKind;
    routeIndex?: number;
    route: TrafficRoute | TrafficTcpRoute;
  } | null>(null);
  const [openedSearchListener, setOpenedSearchListener] = useState<
    string | null
  >(null);
  const [openedSearchRoute, setOpenedSearchRoute] = useState<string | null>(
    null,
  );
  const searchListener = routeListenerSearch(location.search);
  const searchRoute = routeEditSearch(location.search);

  useEffect(() => {
    if (
      !searchListener ||
      searchRoute ||
      openedSearchListener === searchListener ||
      editing ||
      !listeners.length
    )
      return;
    const selected = listenerFromSearch(searchListener, listeners);
    setOpenedSearchListener(searchListener);
    if (!selected) return;
    const kind = listenerRouteKind(selected.listener);
    setEditing({
      bindIndex: selected.bindIndex,
      listenerIndex: selected.listenerIndex,
      kind,
      route: makeRoute(kind),
    });
  }, [editing, listeners, openedSearchListener, searchListener, searchRoute]);

  useEffect(() => {
    if (
      !searchRoute ||
      openedSearchRoute === searchRoute ||
      editing ||
      !routes.length
    )
      return;
    const selected = routeFromSearch(searchRoute, routes);
    setOpenedSearchRoute(searchRoute);
    if (!selected) return;
    setEditing({
      bindIndex: selected.bindIndex,
      listenerIndex: selected.listenerIndex,
      kind: selected.kind,
      routeIndex: selected.routeIndex,
      route: structuredClone(selected.route),
    });
  }, [editing, openedSearchRoute, routes, searchRoute]);

  function openAddRoute(
    bindIndex: number,
    listenerIndex: number,
    listener: TrafficListener,
  ) {
    const kind = listenerRouteKind(listener);
    setEditing({ bindIndex, listenerIndex, kind, route: makeRoute(kind) });
    writeTrafficRouteSearch({ listener: `${bindIndex}:${listenerIndex}` });
  }

  function openEditRoute(context: ReturnType<typeof routeContexts>[number]) {
    setEditing({
      bindIndex: context.bindIndex,
      listenerIndex: context.listenerIndex,
      kind: context.kind,
      routeIndex: context.routeIndex,
      route: structuredClone(context.route),
    });
    writeTrafficRouteSearch({ route: routeSearchValue(context) });
  }

  function closeEditor() {
    setEditing(null);
    setOpenedSearchListener(null);
    setOpenedSearchRoute(null);
    writeTrafficRouteSearch(null, "replace");
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="Traffic Routes"
        description="Match incoming HTTP and TCP traffic and attach inline backends."
        actions={
          <button
            className="button primary"
            type="button"
            disabled={!listeners.length}
            onClick={() => {
              const first = listeners[0];
              openAddRoute(
                first.bindIndex,
                first.listenerIndex,
                first.listener,
              );
            }}
          >
            <Plus size={16} />
            Add route
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
      {stats.invalidListeners ? (
        <StatusBanner
          state="warn"
          title="Some listeners mix HTTP and TCP routes"
        >
          Split mixed listeners before using the route form.
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading traffic routes" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">
            {config.error.message}
          </StatusBanner>
        ) : !routes.length ? (
          <EmptyState
            title="No traffic routes configured"
            description="Add a route under an HTTP or TCP listener."
            action={
              <button
                className="button primary"
                type="button"
                disabled={!listeners.length}
                onClick={() => {
                  const first = listeners[0];
                  if (!first) return;
                  openAddRoute(
                    first.bindIndex,
                    first.listenerIndex,
                    first.listener,
                  );
                }}
              >
                <RouteIcon size={16} />
                Add route
              </button>
            }
          />
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Type</th>
                  <th>Bind</th>
                  <th>Listener</th>
                  <th>Match</th>
                  <th>Backends</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {routes.map((context) => (
                  <tr
                    key={`${context.bindIndex}-${context.listenerIndex}-${context.kind}-${context.routeIndex}`}
                  >
                    <td className="strong">
                      {routeDisplayName(context.route, context.routeIndex)}
                    </td>
                    <td>
                      <span className="badge">
                        {context.kind.toUpperCase()}
                      </span>
                    </td>
                    <td>{context.bind.port}</td>
                    <td>
                      <Link
                        className="table-link"
                        to="/traffic/listeners"
                        search={{
                          drawer: `listener:${context.bindIndex}:${context.listenerIndex}`,
                        }}
                      >
                        {listenerDisplayName(
                          context.listener,
                          context.listenerIndex,
                        )}
                      </Link>
                    </td>
                    <td>
                      {context.kind === "http"
                        ? pathSummary(context.route)
                        : "TCP"}
                    </td>
                    <td>{backendListSummary(context.route.backends)}</td>
                    <td className="row-actions">
                      <Tooltip content="Edit route">
                        <button
                          className="icon-button"
                          type="button"
                          aria-label="Edit route"
                          onClick={() => openEditRoute(context)}
                        >
                          <Pencil size={16} />
                        </button>
                      </Tooltip>
                      <Tooltip content="Delete route">
                        <button
                          className="icon-button danger"
                          type="button"
                          aria-label="Delete route"
                          onClick={() =>
                            update.mutate((next) => {
                              const listener =
                                next.binds?.[context.bindIndex]?.listeners?.[
                                  context.listenerIndex
                                ];
                              if (!listener) return;
                              const routes = routeArray(listener, context.kind);
                              routes.splice(context.routeIndex, 1);
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
        )}
      </Panel>

      {editing ? (
        <RouteEditor
          listeners={listeners}
          editing={editing}
          help={help}
          saving={update.isPending}
          onCancel={closeEditor}
          onSave={(nextEditing) =>
            update.mutate(
              (next) => {
                const listener =
                  next.binds?.[nextEditing.bindIndex]?.listeners?.[
                    nextEditing.listenerIndex
                  ];
                if (!listener) return;
                const routes = routeArray(listener, nextEditing.kind);
                if (typeof nextEditing.routeIndex === "number")
                  routes[nextEditing.routeIndex] = nextEditing.route as never;
                else routes.push(nextEditing.route as never);
              },
              { onSuccess: closeEditor },
            )
          }
        />
      ) : null}
    </div>
  );
}

function RouteEditor(props: {
  listeners: Array<{
    bind: { port: number };
    bindIndex: number;
    listener: TrafficListener;
    listenerIndex: number;
  }>;
  editing: {
    bindIndex: number;
    listenerIndex: number;
    kind: RouteKind;
    routeIndex?: number;
    route: TrafficRoute | TrafficTcpRoute;
  };
  help: SchemaHelp;
  saving: boolean;
  onCancel: () => void;
  onSave: (editing: {
    bindIndex: number;
    listenerIndex: number;
    kind: RouteKind;
    routeIndex?: number;
    route: TrafficRoute | TrafficTcpRoute;
  }) => void;
}) {
  const [listenerKey, setListenerKey] = useState(
    `${props.editing.bindIndex}:${props.editing.listenerIndex}`,
  );
  const [kind, setKind] = useState<RouteKind>(props.editing.kind);
  const [route, setRoute] = useState<TrafficRoute | TrafficTcpRoute>(
    props.editing.route,
  );
  const [error, setError] = useState<string | null>(null);
  const selectedListener = props.listeners.find(
    (item) => `${item.bindIndex}:${item.listenerIndex}` === listenerKey,
  );
  const effectiveKind = selectedListener
    ? listenerRouteKind(selectedListener.listener)
    : kind;
  const preview = cleanRoute(route, effectiveKind);

  function save() {
    const [bindIndex, listenerIndex] = listenerKey.split(":").map(Number);
    if (!selectedListener) {
      setError("Select a listener.");
      return;
    }
    props.onSave({
      bindIndex,
      listenerIndex,
      kind: effectiveKind,
      routeIndex: props.editing.routeIndex,
      route: preview,
    });
  }

  return (
    <Drawer
      title={
        typeof props.editing.routeIndex === "number"
          ? "Edit route"
          : "Add route"
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
            onClick={save}
          >
            <Save size={16} />
            Save route
          </button>
        </div>
      }
    >
      {error ? <StatusBanner state="bad" title={error} /> : null}
      <div className="route-editor-stack">
        {typeof props.editing.routeIndex !== "number" ? (
          <FieldGroup label="Listener" tooltip="Listener that owns this route.">
            <Dropdown
              ariaLabel="Listener"
              value={listenerKey}
              options={props.listeners.map((item) => ({
                value: `${item.bindIndex}:${item.listenerIndex}`,
                label: `:${item.bind.port} · ${listenerDisplayName(item.listener, item.listenerIndex)} · ${listenerRouteKind(item.listener).toUpperCase()}`,
              }))}
              onChange={(value) => {
                setListenerKey(value);
                const nextListener = props.listeners.find(
                  (item) => `${item.bindIndex}:${item.listenerIndex}` === value,
                );
                const nextKind = nextListener
                  ? listenerRouteKind(nextListener.listener)
                  : kind;
                setKind(nextKind);
                setRoute(makeRoute(nextKind));
              }}
            />
          </FieldGroup>
        ) : null}

        <div className="form-grid">
          <Field
            label="Name"
            tooltip={
              effectiveKind === "http"
                ? props.help.field<TrafficRoute>("LocalRoute", "name")
                : props.help.field<TrafficTcpRoute>("LocalTCPRoute", "name")
            }
          >
            <input
              value={route.name ?? ""}
              onChange={(event) =>
                setRoute({ ...route, name: event.target.value })
              }
              placeholder="api"
            />
          </Field>
          <Field
            label="Hostnames"
            tooltip={props.help.field<TrafficRoute>(
              "LocalRoute",
              "hostnames",
              "Comma-separated hostnames. Wildcards are allowed.",
            )}
          >
            <input
              value={(route.hostnames ?? []).join(", ")}
              onChange={(event) =>
                setRoute({ ...route, hostnames: splitList(event.target.value) })
              }
              placeholder="example.com, *.example.com"
            />
          </Field>
        </div>

        {effectiveKind === "http" ? (
          <HttpMatchEditor
            route={route as TrafficRoute}
            help={props.help}
            onChange={setRoute}
          />
        ) : null}

        <RouteBackendsEditor
          kind={effectiveKind}
          help={props.help}
          backends={
            (route.backends ?? []) as Array<
              TrafficRouteBackend | TrafficTcpRouteBackend
            >
          }
          onChange={(backends) =>
            setRoute({ ...route, backends: backends as never })
          }
        />

        <TrafficPolicySection
          title="Route policies"
          schemaRoot={
            effectiveKind === "http" ? "FilterOrPolicy" : "TCPFilterOrPolicy"
          }
          policies={
            route.policies as Record<string, unknown> | null | undefined
          }
          onChange={(policies) => setRoute({ ...route, policies })}
        />

        <details open>
          <summary>Resulting YAML</summary>
          <YamlBlock value={preview} />
        </details>
      </div>
    </Drawer>
  );
}

function HttpMatchEditor(props: {
  route: TrafficRoute;
  help: SchemaHelp;
  onChange: (route: TrafficRoute) => void;
}) {
  const first = props.route.matches?.[0] ?? { path: { pathPrefix: "/" } };
  const path =
    first.path && first.path !== "invalid" ? first.path : { pathPrefix: "/" };
  const pathType =
    "regex" in path ? "regex" : "exact" in path ? "exact" : "pathPrefix";
  const pathValue =
    "regex" in path
      ? path.regex
      : "exact" in path
        ? path.exact
        : path.pathPrefix;

  function updateFirst(next: typeof first) {
    const rest = props.route.matches?.slice(1) ?? [];
    props.onChange({ ...props.route, matches: [next, ...rest] });
  }

  return (
    <>
      <div className="form-grid">
        <FieldGroup
          label="Path match"
          tooltip={props.help.field<GeneratedRouteMatch>("RouteMatch", "path")}
        >
          <EnumSelector
            ariaLabel="Path match"
            value={pathType}
            options={pathTypes.map((value) => ({
              value,
              label: pathLabel(value),
            }))}
            schema={props.help.node([
              "$defs",
              "RouteMatch",
              "properties",
              "path",
            ])}
            onChange={(value) =>
              updateFirst({
                ...first,
                path: { [value]: pathValue || "/" },
              } as typeof first)
            }
          />
        </FieldGroup>
        <Field
          label="Path"
          tooltip={props.help.field<GeneratedRouteMatch>("RouteMatch", "path")}
        >
          <input
            value={pathValue}
            onChange={(event) =>
              updateFirst({
                ...first,
                path: { [pathType]: event.target.value },
              } as typeof first)
            }
            placeholder="/"
          />
        </Field>
        <Field
          label="Method"
          tooltip={props.help.field<GeneratedRouteMatch>(
            "RouteMatch",
            "method",
          )}
        >
          <input
            value={first.method ?? ""}
            onChange={(event) =>
              updateFirst({ ...first, method: event.target.value || undefined })
            }
            placeholder="GET"
          />
        </Field>
      </div>
      <div className="form-grid">
        <HeaderConditionsEditor
          headers={first.headers ?? []}
          onChange={(headers) => updateFirst({ ...first, headers })}
        />
        <QueryConditionsEditor
          query={first.query ?? []}
          onChange={(query) => updateFirst({ ...first, query })}
        />
      </div>
    </>
  );
}

function HeaderConditionsEditor(props: {
  headers: HeaderMatch[];
  onChange: (headers: HeaderMatch[]) => void;
}) {
  return (
    <div className="traffic-match-editor">
      <div className="traffic-match-editor-header">
        <div>
          <h4>Headers</h4>
          <p>Every listed header condition must match.</p>
        </div>
        <button
          className="button small"
          type="button"
          onClick={() =>
            props.onChange([
              ...props.headers,
              { name: "", value: { exact: "" } },
            ])
          }
        >
          <Plus size={16} />
          Add header
        </button>
      </div>
      {props.headers.length ? (
        <div className="match-header-list">
          {props.headers.map((header, index) => (
            <HeaderConditionRow
              key={index}
              header={header}
              onChange={(next) =>
                props.onChange(
                  props.headers.map((item, itemIndex) =>
                    itemIndex === index ? next : item,
                  ),
                )
              }
              onRemove={() =>
                props.onChange(
                  props.headers.filter((_, itemIndex) => itemIndex !== index),
                )
              }
            />
          ))}
        </div>
      ) : (
        <div className="empty-inline">No header conditions.</div>
      )}
    </div>
  );
}

function HeaderConditionRow(props: {
  header: HeaderMatch;
  onChange: (header: HeaderMatch) => void;
  onRemove: () => void;
}) {
  const { mode, text } = matchValueParts(props.header.value);
  const setMode = (regex: boolean) =>
    props.onChange({
      ...props.header,
      value: regex ? { regex: text } : { exact: text },
    });
  const setText = (next: string) =>
    props.onChange({
      ...props.header,
      value: mode === "regex" ? { regex: next } : { exact: next },
    });

  return (
    <div className="header-match-row">
      <div className="condition-inputs">
        <input
          aria-label="Header name"
          value={props.header.name}
          onChange={(event) =>
            props.onChange({ ...props.header, name: event.target.value })
          }
          placeholder="Header name"
        />
        <input
          aria-label="Header value"
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={mode === "regex" ? "Regex value" : "Exact value"}
        />
      </div>
      <div className="condition-actions">
        <label
          className={
            mode === "regex" ? "regex-toggle selected" : "regex-toggle"
          }
        >
          <input
            type="checkbox"
            checked={mode === "regex"}
            onChange={(event) => setMode(event.target.checked)}
          />
          Regex
        </label>
        <Tooltip content="Remove header condition">
          <button
            className="icon-button danger"
            type="button"
            aria-label="Remove header condition"
            onClick={props.onRemove}
          >
            <Trash2 size={15} />
          </button>
        </Tooltip>
      </div>
    </div>
  );
}

function QueryConditionsEditor(props: {
  query: QueryMatch[];
  onChange: (query: QueryMatch[]) => void;
}) {
  return (
    <div className="traffic-match-editor">
      <div className="traffic-match-editor-header">
        <div>
          <h4>Query</h4>
          <p>Every listed query condition must match.</p>
        </div>
        <button
          className="button small"
          type="button"
          onClick={() =>
            props.onChange([...props.query, { name: "", value: { exact: "" } }])
          }
        >
          <Plus size={16} />
          Add query
        </button>
      </div>
      {props.query.length ? (
        <div className="match-header-list">
          {props.query.map((query, index) => (
            <QueryConditionRow
              key={index}
              query={query}
              onChange={(next) =>
                props.onChange(
                  props.query.map((item, itemIndex) =>
                    itemIndex === index ? next : item,
                  ),
                )
              }
              onRemove={() =>
                props.onChange(
                  props.query.filter((_, itemIndex) => itemIndex !== index),
                )
              }
            />
          ))}
        </div>
      ) : (
        <div className="empty-inline">No query conditions.</div>
      )}
    </div>
  );
}

function QueryConditionRow(props: {
  query: QueryMatch;
  onChange: (query: QueryMatch) => void;
  onRemove: () => void;
}) {
  const { mode, text } = matchValueParts(props.query.value);
  const setMode = (regex: boolean) =>
    props.onChange({
      ...props.query,
      value: regex ? { regex: text } : { exact: text },
    });
  const setText = (next: string) =>
    props.onChange({
      ...props.query,
      value: mode === "regex" ? { regex: next } : { exact: next },
    });

  return (
    <div className="header-match-row">
      <div className="condition-inputs">
        <input
          aria-label="Query name"
          value={props.query.name}
          onChange={(event) =>
            props.onChange({ ...props.query, name: event.target.value })
          }
          placeholder="Query name"
        />
        <input
          aria-label="Query value"
          value={text}
          onChange={(event) => setText(event.target.value)}
          placeholder={mode === "regex" ? "Regex value" : "Exact value"}
        />
      </div>
      <div className="condition-actions">
        <label
          className={
            mode === "regex" ? "regex-toggle selected" : "regex-toggle"
          }
        >
          <input
            type="checkbox"
            checked={mode === "regex"}
            onChange={(event) => setMode(event.target.checked)}
          />
          Regex
        </label>
        <Tooltip content="Remove query condition">
          <button
            className="icon-button danger"
            type="button"
            aria-label="Remove query condition"
            onClick={props.onRemove}
          >
            <Trash2 size={15} />
          </button>
        </Tooltip>
      </div>
    </div>
  );
}

type EditableBackendKind =
  | "host"
  | "service"
  | "backend"
  | "dynamic"
  | "routeGroup";
type TrafficBackend = TrafficRouteBackend | TrafficTcpRouteBackend;
type HostBackend = TrafficBackend & { host: string };
type BackendReference = TrafficBackend & { backend: string };
type ServiceBackend = TrafficBackend & {
  service: { name: { namespace: string; hostname: string }; port: number };
};
type RouteGroupBackend = TrafficBackend & { routeGroup: string };

function RouteBackendsEditor(props: {
  kind: RouteKind;
  help: SchemaHelp;
  backends: TrafficBackend[];
  onChange: (backends: TrafficBackend[]) => void;
}) {
  return (
    <div className="traffic-backend-editor">
      <div className="traffic-match-editor-header">
        <div>
          <h4>Backends</h4>
          <p>Traffic that matches this route is forwarded to these targets.</p>
        </div>
        <button
          className="button small"
          type="button"
          onClick={() =>
            props.onChange([...props.backends, makeBackend(props.kind, "host")])
          }
        >
          <Plus size={16} />
          Add backend
        </button>
      </div>
      {props.backends.length ? (
        <div className="route-backend-list">
          {props.backends.map((backend, index) => (
            <RouteBackendRow
              key={index}
              kind={props.kind}
              help={props.help}
              backend={backend}
              onChange={(next) =>
                props.onChange(
                  props.backends.map((item, itemIndex) =>
                    itemIndex === index ? next : item,
                  ),
                )
              }
              onRemove={() =>
                props.onChange(
                  props.backends.filter((_, itemIndex) => itemIndex !== index),
                )
              }
            />
          ))}
        </div>
      ) : (
        <div className="empty-inline">No backends configured.</div>
      )}
    </div>
  );
}

function RouteBackendRow(props: {
  kind: RouteKind;
  help: SchemaHelp;
  backend: TrafficBackend;
  onChange: (backend: TrafficBackend) => void;
  onRemove: () => void;
}) {
  const type = editableBackendKind(props.backend);
  if (
    !type ||
    (props.kind === "tcp" && (type === "dynamic" || type === "routeGroup"))
  ) {
    return (
      <div className="route-backend-row readonly">
        <div>
          <strong>{backendSummary(props.backend)}</strong>
          <span>Unsupported backend shape in this form</span>
        </div>
        <Tooltip content="Remove backend">
          <button
            className="icon-button danger"
            type="button"
            aria-label="Remove backend"
            onClick={props.onRemove}
          >
            <Trash2 size={15} />
          </button>
        </Tooltip>
      </div>
    );
  }

  const backend = props.backend;
  const weight = backendWeight(backend);
  const policyRoot =
    props.kind === "http" ? "LocalBackendPolicies" : "LocalTCPBackendPolicies";
  return (
    <div className="route-backend-row expanded">
      <div className="route-backend-main">
        <div className="route-backend-inputs">
          <FieldGroup
            label="Target type"
            tooltip={props.help.definition(
              props.kind === "http"
                ? "LocalRouteBackend"
                : "LocalTCPRouteBackend",
            )}
          >
            <EnumSelector
              ariaLabel="Backend target type"
              value={type}
              options={backendKindOptions(props.kind)}
              onChange={(value) =>
                props.onChange(makeBackend(props.kind, value, backend))
              }
            />
          </FieldGroup>
          <Field
            label="Weight"
            tooltip={
              props.kind === "http"
                ? props.help.field<TrafficRouteBackend>(
                    "LocalRouteBackend",
                    "weight",
                  )
                : props.help.field<TrafficTcpRouteBackend>(
                    "LocalTCPRouteBackend",
                    "weight",
                  )
            }
          >
            <input
              min={1}
              type="number"
              value={weight}
              onChange={(event) =>
                props.onChange(
                  cleanBackendCommon({
                    ...backend,
                    weight: Number(event.target.value) || 1,
                  } as TrafficBackend),
                )
              }
            />
          </Field>
        </div>
        <BackendTargetFields
          kind={props.kind}
          backend={backend}
          targetKind={type}
          help={props.help}
          onChange={props.onChange}
        />
        <TrafficPolicySection
          title="Backend policies"
          schemaRoot={policyRoot}
          policies={backendPolicies(backend)}
          onChange={(policies) =>
            props.onChange(
              cleanBackendCommon({ ...backend, policies } as TrafficBackend),
            )
          }
        />
      </div>
      <Tooltip content="Remove backend">
        <button
          className="icon-button danger"
          type="button"
          aria-label="Remove backend"
          onClick={props.onRemove}
        >
          <Trash2 size={15} />
        </button>
      </Tooltip>
    </div>
  );
}

function BackendTargetFields(props: {
  kind: RouteKind;
  targetKind: EditableBackendKind;
  backend: TrafficBackend;
  help: SchemaHelp;
  onChange: (backend: TrafficBackend) => void;
}) {
  if (props.targetKind === "host" && isHostBackend(props.backend)) {
    return (
      <div className="route-backend-target-grid single">
        <Field
          label="Host"
          tooltip={
            props.kind === "http"
              ? props.help.field<TrafficRouteBackend>(
                  "LocalRouteBackend",
                  "host",
                )
              : props.help.field<TrafficTcpRouteBackend>(
                  "LocalTCPRouteBackend",
                  "host",
                )
          }
        >
          <input
            value={props.backend.host}
            onChange={(event) =>
              props.onChange(
                cleanBackendCommon({
                  ...props.backend,
                  host: event.target.value.trimStart(),
                }),
              )
            }
            placeholder="localhost:8080"
          />
        </Field>
      </div>
    );
  }

  if (props.targetKind === "backend" && isBackendReference(props.backend)) {
    return (
      <div className="route-backend-target-grid single">
        <Field
          label="Backend reference"
          tooltip={
            props.kind === "http"
              ? props.help.field<TrafficRouteBackend>(
                  "LocalRouteBackend",
                  "backend",
                )
              : props.help.field<TrafficTcpRouteBackend>(
                  "LocalTCPRouteBackend",
                  "backend",
                )
          }
        >
          <input
            value={props.backend.backend}
            onChange={(event) =>
              props.onChange(
                cleanBackendCommon({
                  ...props.backend,
                  backend: event.target.value.trimStart(),
                }),
              )
            }
            placeholder="backend-name"
          />
        </Field>
      </div>
    );
  }

  if (props.targetKind === "service" && isServiceBackend(props.backend)) {
    const service = props.backend.service;
    return (
      <div className="route-backend-target-grid service">
        <Field label="Namespace">
          <input
            value={service.name.namespace}
            onChange={(event) =>
              props.onChange(
                cleanBackendCommon({
                  ...props.backend,
                  service: {
                    ...service,
                    name: { ...service.name, namespace: event.target.value },
                  },
                }),
              )
            }
            placeholder="default"
          />
        </Field>
        <Field label="Hostname">
          <input
            value={service.name.hostname}
            onChange={(event) =>
              props.onChange(
                cleanBackendCommon({
                  ...props.backend,
                  service: {
                    ...service,
                    name: { ...service.name, hostname: event.target.value },
                  },
                }),
              )
            }
            placeholder="api"
          />
        </Field>
        <Field label="Port">
          <input
            type="number"
            min={1}
            max={65535}
            value={service.port}
            onChange={(event) =>
              props.onChange(
                cleanBackendCommon({
                  ...props.backend,
                  service: {
                    ...service,
                    port: Number(event.target.value) || 80,
                  },
                }),
              )
            }
          />
        </Field>
      </div>
    );
  }

  if (props.targetKind === "routeGroup" && isRouteGroupBackend(props.backend)) {
    return (
      <div className="route-backend-target-grid single">
        <Field label="Route group">
          <input
            value={props.backend.routeGroup}
            onChange={(event) =>
              props.onChange(
                cleanBackendCommon({
                  ...props.backend,
                  routeGroup: event.target.value.trimStart(),
                }),
              )
            }
            placeholder="shared-routes"
          />
        </Field>
      </div>
    );
  }

  if (props.targetKind === "dynamic") {
    return (
      <div className="empty-inline">
        Dynamic backend selection is enabled for this backend.
      </div>
    );
  }

  return null;
}

function makeRoute(kind: RouteKind): TrafficRoute | TrafficTcpRoute {
  if (kind === "tcp") return { hostnames: [], backends: [] };
  return {
    hostnames: [],
    matches: [{ path: { pathPrefix: "/" } }],
    backends: [],
  };
}

function cleanRoute(route: TrafficRoute | TrafficTcpRoute, kind: RouteKind) {
  const next = { ...route };
  if (!next.name) delete next.name;
  if (!next.ruleName) delete next.ruleName;
  if (!next.hostnames?.length) delete next.hostnames;
  const backends = (
    (next.backends ?? []) as Array<TrafficRouteBackend | TrafficTcpRouteBackend>
  )
    .map(cleanBackend)
    .filter(backendIsConfigured);
  if (backends.length) next.backends = backends as never;
  else delete next.backends;
  if (!next.policies) delete next.policies;
  if (kind === "http" && !("matches" in next)) {
    return {
      ...next,
      matches: [{ path: { pathPrefix: "/" } }],
    } as TrafficRoute;
  }
  if (kind === "http" && "matches" in next && next.matches) {
    next.matches = next.matches.map(cleanHttpMatch);
  }
  return next;
}

function cleanBackend(backend: TrafficRouteBackend | TrafficTcpRouteBackend) {
  return editableBackendKind(backend) ? cleanBackendCommon(backend) : backend;
}

function backendKindOptions(
  kind: RouteKind,
): Array<EnumSelectorOption<EditableBackendKind>> {
  const base: Array<EnumSelectorOption<EditableBackendKind>> = [
    { value: "host", label: "Host" },
    { value: "service", label: "Service" },
    { value: "backend", label: "Backend reference" },
  ];
  if (kind === "tcp") return base;
  return [
    ...base,
    { value: "dynamic", label: "Dynamic" },
    { value: "routeGroup", label: "Route group" },
  ];
}

function editableBackendKind(
  backend: TrafficBackend,
): EditableBackendKind | null {
  if (!backend || typeof backend !== "object") return null;
  if (isHostBackend(backend)) return "host";
  if (isServiceBackend(backend)) return "service";
  if (isBackendReference(backend)) return "backend";
  if ("dynamic" in backend) return "dynamic";
  if (isRouteGroupBackend(backend)) return "routeGroup";
  return null;
}

function makeBackend(
  kind: RouteKind,
  targetKind: EditableBackendKind,
  previous?: TrafficBackend,
): TrafficBackend {
  const common = backendCommon(previous);
  if (targetKind === "service") {
    return cleanBackendCommon({
      ...common,
      service: {
        name: { namespace: "default", hostname: "" },
        port: 80,
      },
    } as TrafficBackend);
  }
  if (targetKind === "backend")
    return cleanBackendCommon({ ...common, backend: "" } as TrafficBackend);
  if (targetKind === "dynamic" && kind === "http")
    return cleanBackendCommon({ ...common, dynamic: {} } as TrafficBackend);
  if (targetKind === "routeGroup" && kind === "http")
    return cleanBackendCommon({ ...common, routeGroup: "" } as TrafficBackend);
  return cleanBackendCommon({ ...common, host: "" } as TrafficBackend);
}

function backendCommon(backend: TrafficBackend | undefined) {
  const common: { weight?: number; policies?: Record<string, unknown> | null } =
    {};
  if (!backend || typeof backend !== "object") return common;
  if (
    typeof backend.weight === "number" &&
    Number.isFinite(backend.weight) &&
    backend.weight !== 1
  )
    common.weight = backend.weight;
  if (backend.policies && typeof backend.policies === "object")
    common.policies = backend.policies as Record<string, unknown>;
  return common;
}

function backendWeight(backend: TrafficBackend) {
  return typeof backend === "object" &&
    backend &&
    typeof backend.weight === "number" &&
    Number.isFinite(backend.weight)
    ? backend.weight
    : 1;
}

function backendPolicies(backend: TrafficBackend) {
  if (
    !backend ||
    typeof backend !== "object" ||
    !backend.policies ||
    typeof backend.policies !== "object"
  )
    return null;
  return backend.policies as Record<string, unknown>;
}

function cleanBackendCommon<T extends TrafficBackend>(backend: T): T {
  if (!backend || typeof backend !== "object") return backend;
  const next = structuredClone(backend) as TrafficBackend;
  if (
    typeof next.weight !== "number" ||
    !Number.isFinite(next.weight) ||
    next.weight === 1
  )
    delete next.weight;
  if (
    !next.policies ||
    (typeof next.policies === "object" &&
      Object.keys(next.policies).length === 0)
  )
    delete next.policies;
  if (isHostBackend(next)) next.host = next.host.trimStart();
  if (isBackendReference(next)) next.backend = next.backend.trimStart();
  if (isRouteGroupBackend(next)) next.routeGroup = next.routeGroup.trimStart();
  if (isServiceBackend(next)) {
    next.service = {
      ...next.service,
      name: {
        namespace: next.service.name.namespace.trim() || "default",
        hostname: next.service.name.hostname.trimStart(),
      },
      port: Number(next.service.port) || 80,
    };
  }
  return next as T;
}

function backendIsConfigured(backend: TrafficBackend) {
  const kind = editableBackendKind(backend);
  if (!kind) return true;
  if (kind === "host" && isHostBackend(backend))
    return Boolean(backend.host.trim());
  if (kind === "backend" && isBackendReference(backend))
    return Boolean(backend.backend.trim());
  if (kind === "routeGroup" && isRouteGroupBackend(backend))
    return Boolean(backend.routeGroup.trim());
  if (kind === "service" && isServiceBackend(backend))
    return Boolean(
      backend.service.name.hostname.trim() && backend.service.port,
    );
  return true;
}

function isHostBackend(backend: TrafficBackend): backend is HostBackend {
  return Boolean(
    backend &&
    typeof backend === "object" &&
    "host" in backend &&
    typeof backend.host === "string",
  );
}

function isBackendReference(
  backend: TrafficBackend,
): backend is BackendReference {
  return Boolean(
    backend &&
    typeof backend === "object" &&
    "backend" in backend &&
    typeof backend.backend === "string",
  );
}

function isRouteGroupBackend(
  backend: TrafficBackend,
): backend is RouteGroupBackend {
  return Boolean(
    backend &&
    typeof backend === "object" &&
    "routeGroup" in backend &&
    typeof backend.routeGroup === "string",
  );
}

function isServiceBackend(backend: TrafficBackend): backend is ServiceBackend {
  if (!backend || typeof backend !== "object" || !("service" in backend))
    return false;
  const service = backend.service;
  if (
    !service ||
    typeof service !== "object" ||
    !("name" in service) ||
    !("port" in service)
  )
    return false;
  const name = service.name;
  return Boolean(
    name &&
    typeof name === "object" &&
    "namespace" in name &&
    "hostname" in name &&
    typeof name.namespace === "string" &&
    typeof name.hostname === "string" &&
    typeof service.port === "number",
  );
}

function cleanHttpMatch(match: HttpMatch): HttpMatch {
  const next = { ...match };
  const headers = (next.headers ?? []).filter((header) => header.name.trim());
  const query = (next.query ?? []).filter((item) => item.name.trim());
  if (headers.length) next.headers = headers;
  else delete next.headers;
  if (query.length) next.query = query;
  else delete next.query;
  if (!next.method) delete next.method;
  return next;
}

function backendListSummary(
  backends:
    | Array<TrafficRouteBackend | TrafficTcpRouteBackend>
    | null
    | undefined,
) {
  if (!backends?.length) return "No backends";
  return backends.map(backendSummary).join(", ");
}

function listenerRouteKind(listener: TrafficListener): RouteKind {
  return listener.protocol === "TCP" || listener.protocol === "TLS"
    ? "tcp"
    : "http";
}

function routeListenerSearch(search: unknown) {
  if (!search || typeof search !== "object") return null;
  const value = (search as { listener?: unknown }).listener;
  return typeof value === "string" && value.trim() ? value : null;
}

function routeEditSearch(search: unknown) {
  if (!search || typeof search !== "object") return null;
  const value = (search as { route?: unknown }).route;
  return typeof value === "string" && value.trim() ? value : null;
}

function listenerFromSearch(
  value: string,
  listeners: Array<{
    bindIndex: number;
    listenerIndex: number;
    listener: TrafficListener;
  }>,
) {
  const [bindIndex, listenerIndex] = value.split(":").map(Number);
  if (!Number.isInteger(bindIndex) || !Number.isInteger(listenerIndex))
    return undefined;
  return listeners.find(
    (item) =>
      item.bindIndex === bindIndex && item.listenerIndex === listenerIndex,
  );
}

function routeFromSearch(
  value: string,
  routes: ReturnType<typeof routeContexts>,
) {
  const [bindIndex, listenerIndex, kind, routeIndex] = value.split(":");
  const parsedBindIndex = Number(bindIndex);
  const parsedListenerIndex = Number(listenerIndex);
  const parsedRouteIndex = Number(routeIndex);
  if (
    !Number.isInteger(parsedBindIndex) ||
    !Number.isInteger(parsedListenerIndex) ||
    !Number.isInteger(parsedRouteIndex)
  )
    return undefined;
  if (kind !== "http" && kind !== "tcp") return undefined;
  return routes.find(
    (item) =>
      item.bindIndex === parsedBindIndex &&
      item.listenerIndex === parsedListenerIndex &&
      item.kind === kind &&
      item.routeIndex === parsedRouteIndex,
  );
}

function routeSearchValue(context: ReturnType<typeof routeContexts>[number]) {
  return `${context.bindIndex}:${context.listenerIndex}:${context.kind}:${context.routeIndex}`;
}

function writeTrafficRouteSearch(
  next: { listener?: string; route?: string } | null,
  mode: "push" | "replace" = "push",
) {
  const url = new URL(window.location.href);
  url.searchParams.delete("listener");
  url.searchParams.delete("route");
  if (next?.listener) url.searchParams.set("listener", next.listener);
  if (next?.route) url.searchParams.set("route", next.route);
  const target = `${url.pathname}${url.search}${url.hash}`;
  if (mode === "replace") window.history.replaceState(null, "", target);
  else window.history.pushState(null, "", target);
}

function pathLabel(value: string) {
  if (value === "pathPrefix") return "Prefix";
  if (value === "exact") return "Exact";
  return "Regex";
}

function splitList(value: string) {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function matchValueParts(value: unknown): {
  mode: "exact" | "regex";
  text: string;
} {
  if (value === "invalid") return { mode: "exact", text: "" };
  if (!value || typeof value !== "object") return { mode: "exact", text: "" };
  if ("regex" in value)
    return { mode: "regex", text: String(value.regex ?? "") };
  if ("exact" in value)
    return { mode: "exact", text: String(value.exact ?? "") };
  return { mode: "exact", text: "" };
}
