import { useLocation } from "../_adapters/_router";
import { Pencil, Plus, Route as RouteIcon, Save, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Drawer, Dropdown, EmptyState, Field, FieldGroup, PageHeader, Panel, StatusBanner, Tooltip, YamlBlock } from "../components/Primitives";
import { useGatewayConfig, useUpdateConfig } from "../_adapters/hooks";
import { listenerDisplayName, pathSummary, routeArray, routeContexts, routeDisplayName, trafficStats, type RouteKind } from "../traffic";
import type { TrafficListener, TrafficRoute, TrafficTcpRoute } from "../types";
import { TrafficPolicySection } from "./traffic/TrafficPolicySection";

const pathTypes = ["pathPrefix", "exact", "regex"] as const;
type HttpMatch = NonNullable<TrafficRoute["matches"]>[number];
type HeaderMatch = NonNullable<HttpMatch["headers"]>[number];
type QueryMatch = NonNullable<HttpMatch["query"]>[number];

export function TrafficRoutesPage() {
  const location = useLocation();
  const config = useGatewayConfig();
  const update = useUpdateConfig();
  const routes = useMemo(() => routeContexts(config.data), [config.data]);
  const listeners = useMemo(() => (config.data?.binds ?? []).flatMap((bind, bindIndex) =>
    bind.listeners.map((listener, listenerIndex) => ({ bind, bindIndex, listener, listenerIndex })),
  ), [config.data]);
  const stats = trafficStats(config.data);
  const [editing, setEditing] = useState<{
    bindIndex: number;
    listenerIndex: number;
    kind: RouteKind;
    routeIndex?: number;
    route: TrafficRoute | TrafficTcpRoute;
  } | null>(null);
  const [openedSearchListener, setOpenedSearchListener] = useState<string | null>(null);
  const searchListener = routeListenerSearch(location.search);

  useEffect(() => {
    if (!searchListener || openedSearchListener === searchListener || editing || !listeners.length) return;
    const selected = listenerFromSearch(searchListener, listeners);
    setOpenedSearchListener(searchListener);
    if (!selected) return;
    const kind = listenerRouteKind(selected.listener);
    setEditing({ bindIndex: selected.bindIndex, listenerIndex: selected.listenerIndex, kind, route: makeRoute(kind) });
  }, [editing, listeners, openedSearchListener, searchListener]);

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
              const kind = listenerRouteKind(first.listener);
              setEditing({ bindIndex: first.bindIndex, listenerIndex: first.listenerIndex, kind, route: makeRoute(kind) });
            }}
          >
            <Plus size={16} />
            Add route
          </button>
        }
      />

      {update.isError ? <StatusBanner state="bad" title="Save failed">{update.error!.message}</StatusBanner> : null}
      {update.isSuccess ? <StatusBanner state="ok" title="Configuration saved" /> : null}
      {stats.invalidListeners ? (
        <StatusBanner state="warn" title="Some listeners mix HTTP and TCP routes">
          Split mixed listeners before using the route form.
        </StatusBanner>
      ) : null}

      <Panel>
        {config.isLoading ? (
          <StatusBanner state="loading" title="Loading traffic routes" />
        ) : config.isError ? (
          <StatusBanner state="bad" title="Configuration API unavailable">{config.error!.message}</StatusBanner>
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
                  const kind = listenerRouteKind(first.listener);
                  setEditing({ bindIndex: first.bindIndex, listenerIndex: first.listenerIndex, kind, route: makeRoute(kind) });
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
                  <tr key={`${context.bindIndex}-${context.listenerIndex}-${context.kind}-${context.routeIndex}`}>
                    <td className="strong">{routeDisplayName(context.route, context.routeIndex)}</td>
                    <td><span className="badge">{context.kind.toUpperCase()}</span></td>
                    <td>{context.bind.port}</td>
                    <td>{listenerDisplayName(context.listener, context.listenerIndex)}</td>
                    <td>{context.kind === "http" ? pathSummary(context.route) : "TCP"}</td>
                    <td>{context.route.backends?.length ?? 0}</td>
                    <td className="row-actions">
                      <Tooltip content="Edit route">
                        <button className="icon-button" type="button" aria-label="Edit route" onClick={() => setEditing({
                          bindIndex: context.bindIndex,
                          listenerIndex: context.listenerIndex,
                          kind: context.kind,
                          routeIndex: context.routeIndex,
                          route: structuredClone(context.route),
                        })}>
                          <Pencil size={16} />
                        </button>
                      </Tooltip>
                      <Tooltip content="Delete route">
                        <button className="icon-button danger" type="button" aria-label="Delete route" onClick={() => update.mutate((next) => {
                          const listener = next.binds?.[context.bindIndex]?.listeners?.[context.listenerIndex];
                          if (!listener) return;
                          const routes = routeArray(listener, context.kind);
                          routes.splice(context.routeIndex, 1);
                        })}>
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
          saving={update.isPending}
          onCancel={() => setEditing(null)}
          onSave={(nextEditing) => update.mutate((next) => {
            const listener = next.binds?.[nextEditing.bindIndex]?.listeners?.[nextEditing.listenerIndex];
            if (!listener) return;
            const routes = routeArray(listener, nextEditing.kind);
            if (typeof nextEditing.routeIndex === "number") routes[nextEditing.routeIndex] = nextEditing.route as never;
            else routes.push(nextEditing.route as never);
          }, { onSuccess: () => setEditing(null) })}
        />
      ) : null}
    </div>
  );
}

function RouteEditor(props: {
  listeners: Array<{ bind: { port: number }; bindIndex: number; listener: TrafficListener; listenerIndex: number }>;
  editing: { bindIndex: number; listenerIndex: number; kind: RouteKind; routeIndex?: number; route: TrafficRoute | TrafficTcpRoute };
  saving: boolean;
  onCancel: () => void;
  onSave: (editing: { bindIndex: number; listenerIndex: number; kind: RouteKind; routeIndex?: number; route: TrafficRoute | TrafficTcpRoute }) => void;
}) {
  const [listenerKey, setListenerKey] = useState(`${props.editing.bindIndex}:${props.editing.listenerIndex}`);
  const [kind, setKind] = useState<RouteKind>(props.editing.kind);
  const [route, setRoute] = useState<TrafficRoute | TrafficTcpRoute>(props.editing.route);
  const [error, setError] = useState<string | null>(null);
  const selectedListener = props.listeners.find((item) => `${item.bindIndex}:${item.listenerIndex}` === listenerKey);
  const effectiveKind = selectedListener ? listenerRouteKind(selectedListener.listener) : kind;
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
      title={typeof props.editing.routeIndex === "number" ? "Edit route" : "Add route"}
      onClose={props.onCancel}
      footer={
        <div className="button-row">
          <button className="button" type="button" onClick={props.onCancel}>Cancel</button>
          <button className="button primary" type="button" disabled={props.saving} onClick={save}>
            <Save size={16} />
            Save route
          </button>
        </div>
      }
    >
      {error ? <StatusBanner state="bad" title={error} /> : null}
      {typeof props.editing.routeIndex !== "number" ? (
        <FieldGroup label="Listener">
          <Dropdown
            ariaLabel="Listener"
            value={listenerKey}
            options={props.listeners.map((item) => ({
              value: `${item.bindIndex}:${item.listenerIndex}`,
              label: `:${item.bind.port} · ${listenerDisplayName(item.listener, item.listenerIndex)} · ${listenerRouteKind(item.listener).toUpperCase()}`,
            }))}
            onChange={(value) => {
              setListenerKey(value);
              const nextListener = props.listeners.find((item) => `${item.bindIndex}:${item.listenerIndex}` === value);
              const nextKind = nextListener ? listenerRouteKind(nextListener.listener) : kind;
              setKind(nextKind);
              setRoute(makeRoute(nextKind));
            }}
          />
        </FieldGroup>
      ) : null}

      <div className="form-grid">
        <Field label="Name">
          <input value={route.name ?? ""} onChange={(event) => setRoute({ ...route, name: event.target.value })} placeholder="api" />
        </Field>
        <Field label="Hostnames" tooltip="Comma-separated hostnames. Wildcards are allowed.">
          <input value={(route.hostnames ?? []).join(", ")} onChange={(event) => setRoute({ ...route, hostnames: splitList(event.target.value) })} placeholder="example.com, *.example.com" />
        </Field>
      </div>

      {effectiveKind === "http" ? <HttpMatchEditor route={route as TrafficRoute} onChange={setRoute} /> : null}

      <TrafficPolicySection
        title="Route policies"
        schemaRoot={effectiveKind === "http" ? "FilterOrPolicy" : "TCPFilterOrPolicy"}
        policies={route.policies as Record<string, unknown> | null | undefined}
        onChange={(policies) => setRoute({ ...route, policies })}
      />

      <details open>
        <summary>Resulting YAML</summary>
        <YamlBlock value={preview} />
      </details>
    </Drawer>
  );
}

function HttpMatchEditor(props: { route: TrafficRoute; onChange: (route: TrafficRoute) => void }) {
  const first = props.route.matches?.[0] ?? { path: { pathPrefix: "/" } };
  const path = first.path && first.path !== "invalid" ? first.path : { pathPrefix: "/" };
  const pathType = "regex" in path ? "regex" : "exact" in path ? "exact" : "pathPrefix";
  const pathValue = "regex" in path ? path.regex : "exact" in path ? path.exact : path.pathPrefix;

  function updateFirst(next: typeof first) {
    props.onChange({ ...props.route, matches: [next] });
  }

  return (
    <>
      <div className="form-grid">
        <FieldGroup label="Path match">
          <Dropdown
            ariaLabel="Path match"
            value={pathType}
            options={pathTypes.map((value) => ({ value, label: pathLabel(value) }))}
            onChange={(value) => updateFirst({ ...first, path: { [value]: pathValue || "/" } } as typeof first)}
          />
        </FieldGroup>
        <Field label="Path">
          <input value={pathValue} onChange={(event) => updateFirst({ ...first, path: { [pathType]: event.target.value } } as typeof first)} placeholder="/" />
        </Field>
        <Field label="Method">
          <input value={first.method ?? ""} onChange={(event) => updateFirst({ ...first, method: event.target.value || undefined })} placeholder="GET" />
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
        <button className="button small" type="button" onClick={() => props.onChange([...props.headers, { name: "", value: { exact: "" } }])}>
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
              onChange={(next) => props.onChange(props.headers.map((item, itemIndex) => itemIndex === index ? next : item))}
              onRemove={() => props.onChange(props.headers.filter((_, itemIndex) => itemIndex !== index))}
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
  const setMode = (regex: boolean) => props.onChange({ ...props.header, value: regex ? { regex: text } : { exact: text } });
  const setText = (next: string) => props.onChange({ ...props.header, value: mode === "regex" ? { regex: next } : { exact: next } });

  return (
    <div className="header-match-row">
      <input aria-label="Header name" value={props.header.name} onChange={(event) => props.onChange({ ...props.header, name: event.target.value })} placeholder="Header name" />
      <input aria-label="Header value" value={text} onChange={(event) => setText(event.target.value)} placeholder={mode === "regex" ? "Regex value" : "Exact value"} />
      <label className={mode === "regex" ? "regex-toggle selected" : "regex-toggle"}>
        <input type="checkbox" checked={mode === "regex"} onChange={(event) => setMode(event.target.checked)} />
        Regex
      </label>
      <Tooltip content="Remove header condition">
        <button className="icon-button danger" type="button" aria-label="Remove header condition" onClick={props.onRemove}>
          <Trash2 size={15} />
        </button>
      </Tooltip>
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
        <button className="button small" type="button" onClick={() => props.onChange([...props.query, { name: "", value: { exact: "" } }])}>
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
              onChange={(next) => props.onChange(props.query.map((item, itemIndex) => itemIndex === index ? next : item))}
              onRemove={() => props.onChange(props.query.filter((_, itemIndex) => itemIndex !== index))}
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
  const setMode = (regex: boolean) => props.onChange({ ...props.query, value: regex ? { regex: text } : { exact: text } });
  const setText = (next: string) => props.onChange({ ...props.query, value: mode === "regex" ? { regex: next } : { exact: next } });

  return (
    <div className="header-match-row">
      <input aria-label="Query name" value={props.query.name} onChange={(event) => props.onChange({ ...props.query, name: event.target.value })} placeholder="Query name" />
      <input aria-label="Query value" value={text} onChange={(event) => setText(event.target.value)} placeholder={mode === "regex" ? "Regex value" : "Exact value"} />
      <label className={mode === "regex" ? "regex-toggle selected" : "regex-toggle"}>
        <input type="checkbox" checked={mode === "regex"} onChange={(event) => setMode(event.target.checked)} />
        Regex
      </label>
      <Tooltip content="Remove query condition">
        <button className="icon-button danger" type="button" aria-label="Remove query condition" onClick={props.onRemove}>
          <Trash2 size={15} />
        </button>
      </Tooltip>
    </div>
  );
}

function makeRoute(kind: RouteKind): TrafficRoute | TrafficTcpRoute {
  if (kind === "tcp") return { hostnames: [], backends: [] };
  return { hostnames: [], matches: [{ path: { pathPrefix: "/" } }], backends: [] };
}

function cleanRoute(route: TrafficRoute | TrafficTcpRoute, kind: RouteKind) {
  const next = { ...route };
  if (!next.name) delete next.name;
  if (!next.ruleName) delete next.ruleName;
  if (!next.hostnames?.length) delete next.hostnames;
  if (!next.backends) next.backends = [];
  if (!next.policies) delete next.policies;
  if (kind === "http" && !("matches" in next)) {
    return { ...next, matches: [{ path: { pathPrefix: "/" } }] } as TrafficRoute;
  }
  if (kind === "http" && "matches" in next && next.matches) {
    next.matches = next.matches.map(cleanHttpMatch);
  }
  return next;
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

function listenerRouteKind(listener: TrafficListener): RouteKind {
  return listener.protocol === "TCP" || listener.protocol === "TLS" ? "tcp" : "http";
}

function routeListenerSearch(search: unknown) {
  if (!search || typeof search !== "object") return null;
  const value = (search as { listener?: unknown }).listener;
  return typeof value === "string" && value.trim() ? value : null;
}

function listenerFromSearch(
  value: string,
  listeners: Array<{ bindIndex: number; listenerIndex: number; listener: TrafficListener }>,
) {
  const [bindIndex, listenerIndex] = value.split(":").map(Number);
  if (!Number.isInteger(bindIndex) || !Number.isInteger(listenerIndex)) return undefined;
  return listeners.find((item) => item.bindIndex === bindIndex && item.listenerIndex === listenerIndex);
}

function pathLabel(value: string) {
  if (value === "pathPrefix") return "Prefix";
  if (value === "exact") return "Exact";
  return "Regex";
}

function splitList(value: string) {
  return value.split(",").map((item) => item.trim()).filter(Boolean);
}

function matchValueParts(value: unknown): { mode: "exact" | "regex"; text: string } {
  if (value === "invalid") return { mode: "exact", text: "" };
  if (!value || typeof value !== "object") return { mode: "exact", text: "" };
  if ("regex" in value) return { mode: "regex", text: String(value.regex ?? "") };
  if ("exact" in value) return { mode: "exact", text: String(value.exact ?? "") };
  return { mode: "exact", text: "" };
}
