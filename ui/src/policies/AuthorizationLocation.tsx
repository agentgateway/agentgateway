import { KeyRound, SlidersHorizontal, X } from "lucide-react";
import { EnumSelector } from "../components/EnumSelector";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import { Field, FieldGroup } from "../components/Primitives";
import type { SchemaHelp } from "../schemaHelp";
import type { AuthorizationLocation } from "../gateway-config";
import { AdvancedSettingPanel, AdvancedSettingRow } from "./PolicyLayout";

type LocationMode =
  | "default"
  | "header"
  | "queryParameter"
  | "cookie"
  | "expression";

export type AuthorizationLocationDraft = {
  mode: LocationMode;
  headerName: string;
  headerPrefix: string;
  queryName: string;
  cookieName: string;
  expression: string;
};

export function authorizationLocationFrom(
  value: unknown,
  defaults: { headerName?: string; headerPrefix?: string } = {},
): AuthorizationLocationDraft {
  const draft: AuthorizationLocationDraft = {
    mode: "default",
    headerName: defaults.headerName ?? "authorization",
    headerPrefix: defaults.headerPrefix ?? "Bearer ",
    queryName: "access_token",
    cookieName: "session",
    expression: "",
  };
  if (!value || typeof value !== "object" || Array.isArray(value)) return draft;
  const location = value as Record<string, unknown>;
  if (location.header && typeof location.header === "object") {
    const header = location.header as Record<string, unknown>;
    return {
      ...draft,
      mode: "header",
      headerName: String(header.name ?? ""),
      headerPrefix: typeof header.prefix === "string" ? header.prefix : "",
    };
  }
  if (location.queryParameter && typeof location.queryParameter === "object") {
    const query = location.queryParameter as Record<string, unknown>;
    return {
      ...draft,
      mode: "queryParameter",
      queryName: String(query.name ?? ""),
    };
  }
  if (location.cookie && typeof location.cookie === "object") {
    const cookie = location.cookie as Record<string, unknown>;
    return {
      ...draft,
      mode: "cookie",
      cookieName: String(cookie.name ?? ""),
    };
  }
  if (typeof location.expression === "string") {
    return { ...draft, mode: "expression", expression: location.expression };
  }
  return draft;
}

export function authorizationLocationToValue(
  draft: AuthorizationLocationDraft,
): AuthorizationLocation | undefined {
  switch (draft.mode) {
    case "default":
      return undefined;
    case "header":
      return draft.headerName.trim()
        ? {
            header: {
              name: draft.headerName.trim(),
              prefix: draft.headerPrefix || undefined,
            },
          }
        : undefined;
    case "queryParameter":
      return draft.queryName.trim()
        ? { queryParameter: { name: draft.queryName.trim() } }
        : undefined;
    case "cookie":
      return draft.cookieName.trim()
        ? { cookie: { name: draft.cookieName.trim() } }
        : undefined;
    case "expression":
      return draft.expression.trim()
        ? { expression: draft.expression.trim() }
        : undefined;
  }
}

export function CredentialLocationSetting(props: {
  value: AuthorizationLocationDraft;
  onChange: (value: AuthorizationLocationDraft) => void;
  help: SchemaHelp;
  defaultDescription?: string;
  description?: string;
  allowExpression?: boolean;
}) {
  if (props.value.mode === "default") {
    return (
      <AdvancedSettingRow
        icon={<KeyRound size={17} />}
        title="Credential location"
        description={
          props.defaultDescription ??
          "By default, callers send Authorization: Bearer token."
        }
        action={
          <button
            className="button compact-action"
            type="button"
            onClick={() => props.onChange({ ...props.value, mode: "header" })}
          >
            <SlidersHorizontal size={15} />
            Customize
          </button>
        }
      />
    );
  }

  return (
    <AdvancedSettingPanel
      icon={<KeyRound size={17} />}
      title="Credential location"
      description={
        props.description ?? "Override where this policy reads the credential."
      }
      action={
        <button
          className="button"
          type="button"
          onClick={() => props.onChange({ ...props.value, mode: "default" })}
        >
          <X size={15} />
          Use default
        </button>
      }
    >
      <div className="location-override-panel">
        <FieldGroup
          label="Location type"
          tooltip={props.help.definition(
            "AuthorizationLocation",
            "Choose where the credential is read from or written to.",
          )}
        >
          <EnumSelector
            ariaLabel="Location type"
            value={props.value.mode}
            options={[
              { value: "header", label: "Header" },
              { value: "queryParameter", label: "Query parameter" },
              { value: "cookie", label: "Cookie" },
              ...(props.allowExpression !== false ||
              props.value.mode === "expression"
                ? [{ value: "expression" as const, label: "CEL expression" }]
                : []),
            ]}
            schema={props.help.node(["$defs", "AuthorizationLocation"])}
            onChange={(mode) => props.onChange({ ...props.value, mode })}
          />
        </FieldGroup>
        {props.value.mode === "header" ? (
          <div className="form-grid">
            <Field
              label="Header name"
              tooltip={props.help.field<AuthorizationLocation>(
                "AuthorizationLocation",
                "header.name",
              )}
            >
              <input
                value={props.value.headerName}
                onChange={(event) =>
                  props.onChange({
                    ...props.value,
                    headerName: event.target.value,
                  })
                }
                placeholder="authorization"
              />
            </Field>
            <Field
              label="Header prefix"
              tooltip={props.help.field<AuthorizationLocation>(
                "AuthorizationLocation",
                "header.prefix",
              )}
            >
              <input
                value={props.value.headerPrefix}
                onChange={(event) =>
                  props.onChange({
                    ...props.value,
                    headerPrefix: event.target.value,
                  })
                }
                placeholder="Bearer "
              />
            </Field>
          </div>
        ) : null}
        {props.value.mode === "queryParameter" ? (
          <Field
            label="Query parameter name"
            tooltip={props.help.field<AuthorizationLocation>(
              "AuthorizationLocation",
              "queryParameter.name",
            )}
          >
            <input
              value={props.value.queryName}
              onChange={(event) =>
                props.onChange({
                  ...props.value,
                  queryName: event.target.value,
                })
              }
              placeholder="access_token"
            />
          </Field>
        ) : null}
        {props.value.mode === "cookie" ? (
          <Field
            label="Cookie name"
            tooltip={props.help.field<AuthorizationLocation>(
              "AuthorizationLocation",
              "cookie.name",
            )}
          >
            <input
              value={props.value.cookieName}
              onChange={(event) =>
                props.onChange({
                  ...props.value,
                  cookieName: event.target.value,
                })
              }
              placeholder="session"
            />
          </Field>
        ) : null}
        {props.value.mode === "expression" ? (
          <FieldGroup
            label="CEL expression"
            tooltip={props.help.field<AuthorizationLocation>(
              "AuthorizationLocation",
              "expression",
            )}
          >
            {props.allowExpression === false ? (
              <small>
                CEL expressions can extract credentials but cannot insert them.
                Choose Header, Query parameter, or Cookie for backend auth.
              </small>
            ) : null}
            <MiniMonacoEditor
              language="cel"
              value={props.value.expression}
              onChange={(expression) =>
                props.onChange({ ...props.value, expression })
              }
              placeholder='request.headers["authorization"]'
            />
          </FieldGroup>
        ) : null}
      </div>
    </AdvancedSettingPanel>
  );
}
