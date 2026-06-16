import { useState } from "react";
import { Save } from "lucide-react";
import type { SchemaHelp } from "../schemaHelp";
import { Field, FieldGroup } from "../components/Primitives";
import { ListEditor } from "./ListEditor";
import { ResultingYaml } from "./ResultingYaml";
import type { CorsPolicy } from "../types";
import { appendUnique, cleanEmpty, toggleStringSet } from "./policyUtils";

const corsMethods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] as const;

export function CorsPolicyEditor(props: {
  cors: CorsPolicy | null | undefined;
  help: SchemaHelp;
  saving: boolean;
  onSave: (cors: CorsPolicy) => void;
}) {
  const [origins, setOrigins] = useState(props.cors?.allowOrigins ?? []);
  const [allowCredentials, setAllowCredentials] = useState(Boolean(props.cors?.allowCredentials));
  const [maxAge, setMaxAge] = useState(props.cors?.maxAge ?? "");
  const [allMethods, setAllMethods] = useState(Boolean(props.cors?.allowMethods?.includes("*")));
  const [methods, setMethods] = useState(() => new Set((props.cors?.allowMethods ?? ["GET", "POST"]).filter((method) => method !== "*")));
  const [allHeaders, setAllHeaders] = useState(Boolean(props.cors?.allowHeaders?.includes("*") ?? true));
  const [headers, setHeaders] = useState((props.cors?.allowHeaders ?? []).filter((header) => header !== "*"));
  const [exposeHeaders, setExposeHeaders] = useState(props.cors?.exposeHeaders ?? []);
  const policy = buildCorsPolicy({ origins, allMethods, methods, allHeaders, headers, exposeHeaders, allowCredentials, maxAge });

  return (
    <div className="policy-editor-stack">
      <ListEditor
        label="Allowed origins"
        tooltip={props.help.description(["$defs", "CorsSerde", "properties", "allowOrigins"])}
        values={origins}
        placeholder="http://localhost:19000"
        onChange={setOrigins}
        actions={
          <button className="button" type="button" onClick={() => setOrigins((current) => appendUnique(current, window.location.origin))}>
            Add current origin
          </button>
        }
      />
      <FieldGroup label="Allowed methods" tooltip={props.help.description(["$defs", "CorsSerde", "properties", "allowMethods"])}>
        <div className="method-grid">
          <button className={allMethods ? "choice-pill active" : "choice-pill"} type="button" onClick={() => setAllMethods((current) => !current)}>
            ALL
          </button>
          {corsMethods.map((method) => (
            <button
              className={!allMethods && methods.has(method) ? "choice-pill active" : "choice-pill"}
              type="button"
              disabled={allMethods}
              key={method}
              onClick={() => setMethods((current) => toggleStringSet(current, method))}
            >
              {method}
            </button>
          ))}
        </div>
      </FieldGroup>
      <FieldGroup label="Allowed headers" tooltip={props.help.description(["$defs", "CorsSerde", "properties", "allowHeaders"])}>
        <label className="config-option-row">
          <input type="checkbox" checked={allHeaders} onChange={(event) => setAllHeaders(event.target.checked)} />
          <span>
            <strong>Allow all request headers</strong>
            <small>Accept any request header in browser preflight checks</small>
          </span>
        </label>
      </FieldGroup>
      {!allHeaders ? (
        <ListEditor
          label="Header allowlist"
          values={headers}
          placeholder="authorization"
          suggestions={["authorization", "content-type", "mcp-session-id"]}
          onChange={setHeaders}
        />
      ) : null}
      <ListEditor
        label="Expose headers"
        tooltip={props.help.description(["$defs", "CorsSerde", "properties", "exposeHeaders"])}
        values={exposeHeaders}
        placeholder="mcp-session-id"
        suggestions={["mcp-session-id", "x-request-id"]}
        onChange={setExposeHeaders}
      />
      <div className="form-grid">
        <FieldGroup label="Credentials" tooltip={props.help.description(["$defs", "CorsSerde", "properties", "allowCredentials"])}>
          <label className="config-option-row">
            <input type="checkbox" checked={allowCredentials} onChange={(event) => setAllowCredentials(event.target.checked)} />
            <span>
              <strong>Allow credentials</strong>
              <small>Permit browser credentials on CORS requests</small>
            </span>
          </label>
        </FieldGroup>
        <Field label="Max age" tooltip={props.help.description(["$defs", "CorsSerde", "properties", "maxAge"])}>
          <input value={maxAge} onChange={(event) => setMaxAge(event.target.value)} placeholder="24h" />
        </Field>
      </div>
      <ResultingYaml value={policy} />
      <button className="button primary" type="button" disabled={props.saving} onClick={() => props.onSave(policy)}>
        <Save size={16} />
        Save CORS
      </button>
    </div>
  );
}

function buildCorsPolicy(args: {
  origins: string[];
  allMethods: boolean;
  methods: Set<string>;
  allHeaders: boolean;
  headers: string[];
  exposeHeaders: string[];
  allowCredentials: boolean;
  maxAge: string;
}): CorsPolicy {
  return cleanEmpty({
    allowOrigins: args.origins,
    allowMethods: args.allMethods ? ["*"] : Array.from(args.methods),
    allowHeaders: args.allHeaders ? ["*"] : args.headers,
    exposeHeaders: args.exposeHeaders,
    allowCredentials: args.allowCredentials ? true : undefined,
    maxAge: args.maxAge.trim() || undefined,
  }) as CorsPolicy;
}
