import {
  EnumSelector,
  type EnumSelectorOption,
} from "../../components/EnumSelector";
import { FieldGroup } from "../../components/Primitives";
import { cleanEmpty } from "../policyUtils";

// -- Authorization location --

type LocationMode = "unset" | "header" | "query" | "cookie" | "preserved";
export type LocationDraft = {
  mode: LocationMode;
  headerName: string;
  headerPrefix: string;
  queryName: string;
  cookieName: string;
  preserved?: unknown;
};

export function emptyLocation(): LocationDraft {
  return {
    mode: "unset",
    headerName: "",
    headerPrefix: "",
    queryName: "",
    cookieName: "",
  };
}

function locationFromValue(value: unknown): LocationDraft {
  if (!value || typeof value !== "object") return emptyLocation();
  const v = value as Record<string, unknown>;
  if (v.header && typeof v.header === "object") {
    const h = v.header as Record<string, unknown>;
    return {
      ...emptyLocation(),
      mode: "header",
      headerName: String(h.name ?? ""),
      headerPrefix: typeof h.prefix === "string" ? h.prefix : "",
    };
  }
  if (v.queryParameter && typeof v.queryParameter === "object") {
    const q = v.queryParameter as Record<string, unknown>;
    return {
      ...emptyLocation(),
      mode: "query",
      queryName: String(q.name ?? ""),
    };
  }
  if (v.cookie && typeof v.cookie === "object") {
    const c = v.cookie as Record<string, unknown>;
    return {
      ...emptyLocation(),
      mode: "cookie",
      cookieName: String(c.name ?? ""),
    };
  }
  // CEL expression sources aren't editable structurally; preserve as-is.
  return { ...emptyLocation(), mode: "preserved", preserved: value };
}

function locationToValue(draft: LocationDraft): unknown {
  switch (draft.mode) {
    case "unset":
      return undefined;
    case "header":
      return draft.headerName.trim()
        ? {
            header: cleanEmpty({
              name: draft.headerName.trim(),
              prefix: draft.headerPrefix.trim() || undefined,
            }),
          }
        : undefined;
    case "query":
      return draft.queryName.trim()
        ? { queryParameter: { name: draft.queryName.trim() } }
        : undefined;
    case "cookie":
      return draft.cookieName.trim()
        ? { cookie: { name: draft.cookieName.trim() } }
        : undefined;
    case "preserved":
      return draft.preserved;
  }
}

export function LocationFields(props: {
  label: string;
  tooltip?: string;
  hint?: string;
  value: LocationDraft;
  onChange: (next: LocationDraft) => void;
  allowUnset?: boolean;
}) {
  const options: Array<EnumSelectorOption<LocationMode>> = [
    ...(props.allowUnset === false
      ? []
      : [{ value: "unset" as const, label: "Default" }]),
    { value: "header", label: "Header" },
    { value: "query", label: "Query parameter" },
    { value: "cookie", label: "Cookie" },
  ];
  return (
    <FieldGroup label={props.label} tooltip={props.tooltip} hint={props.hint}>
      <EnumSelector
        ariaLabel={props.label}
        value={props.value.mode === "preserved" ? "unset" : props.value.mode}
        options={options}
        onChange={(mode) => props.onChange({ ...props.value, mode })}
      />
      {props.value.mode === "preserved" ? (
        <small>
          Uses a CEL expression source; preserved as-is. Switch to Header, Query
          parameter, or Cookie to edit here.
        </small>
      ) : null}
      {props.value.mode === "header" ? (
        <div className="form-grid">
          <input
            value={props.value.headerName}
            onChange={(event) =>
              props.onChange({ ...props.value, headerName: event.target.value })
            }
            placeholder="authorization"
          />
          <input
            value={props.value.headerPrefix}
            onChange={(event) =>
              props.onChange({
                ...props.value,
                headerPrefix: event.target.value,
              })
            }
            placeholder="Bearer  (optional prefix)"
          />
        </div>
      ) : null}
      {props.value.mode === "query" ? (
        <input
          value={props.value.queryName}
          onChange={(event) =>
            props.onChange({ ...props.value, queryName: event.target.value })
          }
          placeholder="access_token"
        />
      ) : null}
      {props.value.mode === "cookie" ? (
        <input
          value={props.value.cookieName}
          onChange={(event) =>
            props.onChange({ ...props.value, cookieName: event.target.value })
          }
          placeholder="session"
        />
      ) : null}
    </FieldGroup>
  );
}

// -- Passthrough draft --

export type PassthroughDraft = { location: LocationDraft };

export function emptyPassthroughDraft(): PassthroughDraft {
  return { location: emptyLocation() };
}

export function passthroughDraftFromValue(
  value: Record<string, unknown>,
): PassthroughDraft {
  return { location: locationFromValue(value.location) };
}

export function passthroughDraftToValue(draft: PassthroughDraft): unknown {
  return (
    cleanEmpty({
      location: locationToValue(draft.location),
    }) ?? {}
  );
}

export function PassthroughFields(props: {
  value: PassthroughDraft;
  onChange: (next: PassthroughDraft) => void;
}) {
  return (
    <LocationFields
      label="Location"
      hint="Optional. Defaults to the Authorization header."
      value={props.value.location}
      onChange={(location) => props.onChange({ ...props.value, location })}
    />
  );
}
