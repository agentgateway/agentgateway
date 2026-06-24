import { useEffect, useMemo, useState } from "react";
import type { KeyboardEvent } from "react";
import { Braces, Cable, Play, RotateCcw } from "lucide-react";
import { sendMcpJsonRpc } from "../api/playgroundApi";
import { applyPlaygroundCors, corsNeedsUpdate, currentOrigin } from "../cors";
import { gatewayEndpoint } from "../gatewayUrls";
import {
  useGatewayConfig,
  useStoredStringState,
  useUpdateConfig,
} from "../hooks";
import {
  extractMcpTools,
  nextRpcId,
  sendInitializedNotification,
  type McpTool,
} from "../mcp";
import { EnumSelector } from "../components/EnumSelector";
import { MiniMonacoEditor } from "../components/MiniMonacoEditor";
import {
  Dropdown,
  Field,
  FieldGroup,
  JsonBlock,
  PageHeader,
  Panel,
  StatusBanner,
} from "../components/Primitives";

type JsonSchema = {
  type?: string | string[];
  properties?: Record<string, JsonSchema>;
  required?: string[];
  description?: string;
  enum?: unknown[];
  items?: JsonSchema;
  default?: unknown;
};

type McpResponse = {
  status: number;
  sessionId: string | null;
  body: unknown;
};

const storageKeys = {
  tool: "mcpPlaygroundTool",
};

export function McpPlaygroundPage() {
  const config = useGatewayConfig();
  const updateConfig = useUpdateConfig();
  const targets = useMemo(() => config.data?.mcp?.targets ?? [], [config.data]);
  const derivedBaseUrl = gatewayEndpoint(
    config.data?.mcp?.port ?? 3000,
    "/mcp",
  );
  const baseUrl = derivedBaseUrl;
  const [initialized, setInitialized] = useState(false);
  const [sessionId, setSessionId] = useState("");
  const [tools, setTools] = useState<McpTool[]>([]);
  const [toolName, setToolName] = useStoredStringState(storageKeys.tool, "");
  const [argumentValues, setArgumentValues] = useState<Record<string, unknown>>(
    {},
  );
  const [argumentsJson, setArgumentsJson] = useState("{}");
  const [bearerToken, setBearerToken] = useState("");
  const [result, setResult] = useState<McpResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState<"initialize" | "call" | null>(null);

  const selectedTool = tools.find((tool) => tool.name === toolName);
  const needsCors = config.data
    ? corsNeedsUpdate(config.data.mcp?.policies?.cors, "mcp")
    : false;

  useEffect(() => {
    localStorage.removeItem("mcpPlaygroundArgs");
  }, []);

  useEffect(() => {
    if (selectedTool?.inputSchema) {
      setArgumentValues(defaultArgumentsFromSchema(selectedTool.inputSchema));
    } else {
      setArgumentValues({});
    }
  }, [selectedTool?.name, selectedTool?.inputSchema]);

  async function initialize() {
    setLoading("initialize");
    setError(null);
    try {
      const response = await sendMcpJsonRpc({
        baseUrl,
        bearerToken,
        body: {
          jsonrpc: "2.0",
          id: nextRpcId(),
          method: "initialize",
          params: {
            protocolVersion: "2025-03-26",
            capabilities: {},
            clientInfo: {
              name: "agentgateway-ui",
              version: "0.1.0",
            },
          },
        },
      });
      setResult(response);
      const nextSessionId = response.sessionId ?? sessionId;
      if (response.sessionId) setSessionId(response.sessionId);
      await sendInitializedNotification(baseUrl, nextSessionId, bearerToken);
      const toolsResponse = await sendMcpJsonRpc({
        baseUrl,
        sessionId: nextSessionId,
        bearerToken,
        body: {
          jsonrpc: "2.0",
          id: nextRpcId(),
          method: "tools/list",
          params: {},
        },
      });
      setResult(toolsResponse);
      if (toolsResponse.sessionId) setSessionId(toolsResponse.sessionId);
      setInitialized(true);
      const nextTools = extractMcpTools(toolsResponse.body);
      setTools(nextTools);
      if (
        nextTools.length > 0 &&
        !nextTools.some((tool) => tool.name === toolName)
      ) {
        setToolName(nextTools[0].name);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "MCP initialize failed");
    } finally {
      setLoading(null);
    }
  }

  async function callTool() {
    setLoading("call");
    setError(null);
    try {
      const toolArguments =
        selectedTool?.inputSchema &&
        schemaHasSimpleProperties(selectedTool.inputSchema)
          ? argumentsFromForm(selectedTool.inputSchema, argumentValues)
          : parseArguments(argumentsJson);
      const response = await sendMcpJsonRpc({
        baseUrl,
        sessionId,
        bearerToken,
        body: {
          jsonrpc: "2.0",
          id: nextRpcId(),
          method: "tools/call",
          params: {
            name: toolName.trim(),
            arguments: toolArguments,
          },
        },
      });
      setResult(response);
      if (response.sessionId) setSessionId(response.sessionId);
    } catch (err) {
      setError(err instanceof Error ? err.message : "MCP tools/call failed");
    } finally {
      setLoading(null);
    }
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="MCP Playground"
        description="Initialize a gateway MCP session, list tools, and call a tool through the MCP listener."
      />
      {needsCors ? (
        <StatusBanner
          state="warn"
          title="Browser access is not allowed"
          action={
            <button
              className="button"
              type="button"
              disabled={updateConfig.isPending}
              onClick={() =>
                updateConfig.mutate((next) => applyPlaygroundCors(next, "mcp"))
              }
            >
              Apply CORS
            </button>
          }
        >
          Add {currentOrigin()} to the MCP CORS policy and expose Mcp-Session-Id
          so this playground can keep a browser session.
        </StatusBanner>
      ) : null}
      {targets.length === 0 ? (
        <StatusBanner state="warn" title="No MCP servers">
          Create an MCP server before testing MCP traffic.
        </StatusBanner>
      ) : null}
      {error ? (
        <StatusBanner state="bad" title="MCP request failed">
          {error}
        </StatusBanner>
      ) : null}
      <section className="two-column wide-left playground-layout">
        <Panel>
          <div className="mcp-session-bar">
            <div>
              <span>Session</span>
              <strong className="mono">
                {initialized ? sessionId || "initialized" : "not initialized"}
              </strong>
            </div>
            {initialized ? (
              <button
                className="button"
                type="button"
                onClick={() => {
                  setInitialized(false);
                  setSessionId("");
                  setTools([]);
                  setToolName("");
                  setResult(null);
                  setError(null);
                }}
              >
                <RotateCcw size={16} />
                Reset
              </button>
            ) : (
              <button
                className="button primary"
                type="button"
                disabled={loading !== null || !baseUrl.trim()}
                onClick={initialize}
              >
                <Cable size={16} />
                Initialize
              </button>
            )}
          </div>

          <details className="schema-details mcp-auth-details">
            <summary>Authorization header</summary>
            <Field label="Bearer token">
              <input
                value={bearerToken}
                type="password"
                className="masked-secret-input"
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="none"
                data-1p-ignore="true"
                data-lpignore="true"
                data-form-type="other"
                name="agw-mcp-playground-bearer-token"
                spellCheck={false}
                onChange={(event) => setBearerToken(event.target.value)}
                placeholder="Optional token"
              />
            </Field>
          </details>

          <FieldGroup label="Tool">
            <Dropdown
              ariaLabel="Tool"
              value={toolName}
              placeholder={
                initialized ? "No tools returned" : "Initialize first"
              }
              searchable
              options={tools.map((tool) => ({
                value: tool.name,
                label: tool.description
                  ? `${tool.name} - ${tool.description}`
                  : tool.name,
                icon: <Braces size={16} />,
                searchText: `${tool.name} ${tool.description ?? ""}`,
              }))}
              onChange={setToolName}
            />
          </FieldGroup>

          {selectedTool?.description ? (
            <div className="tool-description">{selectedTool.description}</div>
          ) : null}
          {selectedTool?.inputSchema ? (
            <details className="schema-details">
              <summary>Input schema</summary>
              <JsonBlock value={selectedTool.inputSchema} />
            </details>
          ) : null}

          {!selectedTool ? (
            <div className="empty-inline">
              Initialize the session and select a tool to configure arguments.
            </div>
          ) : selectedTool.inputSchema &&
            schemaHasSimpleProperties(selectedTool.inputSchema) ? (
            <ToolArgumentsForm
              schema={selectedTool.inputSchema}
              values={argumentValues}
              onChange={setArgumentValues}
              onSubmit={callTool}
            />
          ) : (
            <FieldGroup label="Arguments JSON">
              <MiniMonacoEditor
                language="json"
                value={argumentsJson}
                onChange={setArgumentsJson}
                onSubmit={() => void callTool()}
              />
            </FieldGroup>
          )}
          <div className="button-row mcp-call-actions">
            <button
              className="button primary"
              type="button"
              disabled={
                loading !== null ||
                !initialized ||
                !baseUrl.trim() ||
                !toolName.trim()
              }
              onClick={callTool}
            >
              <Play size={16} />
              Call tool
            </button>
          </div>
        </Panel>
        <Panel className="playground-response-panel">
          <div className="section-heading">
            <h3>Result</h3>
          </div>
          {result ? (
            <McpResultView response={result} />
          ) : (
            <div className="empty-state">
              <h3>No response yet</h3>
              <p>Initialize or send a tool request to inspect MCP behavior.</p>
            </div>
          )}
        </Panel>
      </section>
    </div>
  );
}

function parseArguments(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return {};
  const parsed = JSON.parse(trimmed) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("Arguments must be a JSON object.");
  }
  return parsed;
}

function ToolArgumentsForm(props: {
  schema: unknown;
  values: Record<string, unknown>;
  onChange: (values: Record<string, unknown>) => void;
  onSubmit: () => void;
}) {
  const schema = props.schema as JsonSchema;
  const properties = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  const entries = Object.entries(properties);
  if (entries.length === 0) {
    return (
      <div className="empty-inline">This tool does not declare arguments.</div>
    );
  }
  return (
    <div className="tool-arguments-form">
      {entries.map(([name, property]) => {
        const type = schemaType(property);
        const label = required.has(name) ? `${name} *` : name;
        const value = props.values[name];
        if (property.enum?.length) {
          return (
            <FieldGroup label={label} key={name}>
              <EnumSelector
                ariaLabel={name}
                value={
                  value === undefined
                    ? String(property.enum[0] ?? "")
                    : String(value)
                }
                options={property.enum.map((option) => ({
                  value: String(option),
                  label: String(option),
                }))}
                onChange={(next) =>
                  props.onChange({ ...props.values, [name]: next })
                }
              />
              {property.description ? (
                <small>{property.description}</small>
              ) : null}
            </FieldGroup>
          );
        }
        if (type === "boolean") {
          return (
            <label className="config-option-row" key={name}>
              <input
                type="checkbox"
                checked={Boolean(value)}
                onChange={(event) =>
                  props.onChange({
                    ...props.values,
                    [name]: event.target.checked,
                  })
                }
                onKeyDown={(event) => submitOnModEnter(event, props.onSubmit)}
              />
              <span>
                <strong>{label}</strong>
                {property.description ? (
                  <small>{property.description}</small>
                ) : null}
              </span>
            </label>
          );
        }
        if (type === "number" || type === "integer") {
          return (
            <Field label={label} hint={property.description} key={name}>
              <input
                type="number"
                value={typeof value === "number" ? value : ""}
                onChange={(event) =>
                  props.onChange({
                    ...props.values,
                    [name]:
                      event.target.value === ""
                        ? ""
                        : Number(event.target.value),
                  })
                }
                onKeyDown={(event) => submitOnModEnter(event, props.onSubmit)}
              />
            </Field>
          );
        }
        if (type === "array" || type === "object") {
          return (
            <FieldGroup label={label} hint={property.description} key={name}>
              <MiniMonacoEditor
                language="json"
                value={
                  value === undefined ? "" : JSON.stringify(value, null, 2)
                }
                onChange={(next) =>
                  props.onChange({
                    ...props.values,
                    [name]: parseJsonDraft(next),
                  })
                }
                onSubmit={props.onSubmit}
                placeholder={type === "array" ? "[]" : "{}"}
              />
            </FieldGroup>
          );
        }
        return (
          <Field label={label} hint={property.description} key={name}>
            <input
              value={typeof value === "string" ? value : ""}
              onChange={(event) =>
                props.onChange({ ...props.values, [name]: event.target.value })
              }
              onKeyDown={(event) => submitOnModEnter(event, props.onSubmit)}
            />
          </Field>
        );
      })}
    </div>
  );
}

function submitOnModEnter(
  event: KeyboardEvent<HTMLInputElement | HTMLTextAreaElement>,
  onSubmit: () => void,
) {
  if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
    event.preventDefault();
    onSubmit();
  }
}

function schemaHasSimpleProperties(schema: unknown): schema is JsonSchema {
  return Boolean(
    schema &&
    typeof schema === "object" &&
    !Array.isArray(schema) &&
    (schema as JsonSchema).properties &&
    typeof (schema as JsonSchema).properties === "object",
  );
}

function schemaType(schema: JsonSchema) {
  if (Array.isArray(schema.type))
    return schema.type.find((type) => type !== "null") ?? "string";
  return schema.type ?? "string";
}

function defaultArgumentsFromSchema(schema: unknown) {
  if (!schemaHasSimpleProperties(schema)) return {};
  const values: Record<string, unknown> = {};
  for (const [name, property] of Object.entries(schema.properties ?? {})) {
    if (property.default !== undefined) {
      values[name] = property.default;
      continue;
    }
    if (property.enum?.length) {
      values[name] = String(property.enum[0]);
      continue;
    }
    const type = schemaType(property);
    if (type === "boolean") values[name] = false;
    else if (type === "array") values[name] = [];
    else if (type === "object") values[name] = {};
    else values[name] = "";
  }
  return values;
}

function argumentsFromForm(schema: unknown, values: Record<string, unknown>) {
  if (!schemaHasSimpleProperties(schema)) return values;
  const next: Record<string, unknown> = {};
  for (const [name, property] of Object.entries(schema.properties ?? {})) {
    const value = values[name];
    const type = schemaType(property);
    if (value === "" || value === undefined) continue;
    if (
      (type === "number" || type === "integer") &&
      typeof value === "number" &&
      Number.isFinite(value)
    ) {
      next[name] = value;
    } else if (type !== "number" && type !== "integer") {
      next[name] = value;
    }
  }
  return next;
}

function parseJsonDraft(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return "";
  try {
    return JSON.parse(trimmed) as unknown;
  } catch {
    return value;
  }
}

function McpResultView(props: { response: McpResponse }) {
  const payload = responsePayload(props.response.body);
  const result =
    payload && typeof payload === "object"
      ? (payload as { result?: unknown }).result
      : undefined;
  const error =
    payload && typeof payload === "object"
      ? (payload as { error?: unknown }).error
      : undefined;
  const tools = extractMcpTools(props.response.body);
  const content =
    result &&
    typeof result === "object" &&
    Array.isArray((result as { content?: unknown }).content)
      ? (result as { content: unknown[] }).content
      : [];
  const structuredContent =
    result && typeof result === "object"
      ? (result as { structuredContent?: unknown }).structuredContent
      : undefined;

  return (
    <div className="mcp-result-view">
      <div className="mcp-result-status">
        <span className={error ? "badge bad" : "badge ok"}>
          {error ? "error" : `HTTP ${props.response.status}`}
        </span>
        {props.response.sessionId ? (
          <span className="mono">{props.response.sessionId}</span>
        ) : null}
      </div>

      {error ? (
        <div className="mcp-result-card">
          <strong>Error</strong>
          <JsonBlock value={error} />
        </div>
      ) : tools.length > 0 ? (
        <div className="mcp-result-card">
          <strong>{tools.length} tools discovered</strong>
          <div className="mcp-tool-list">
            {tools.map((tool) => (
              <div className="mcp-tool-row" key={tool.name}>
                <span className="config-chip">
                  <span>{tool.name}</span>
                </span>
                {tool.description ? <small>{tool.description}</small> : null}
              </div>
            ))}
          </div>
        </div>
      ) : content.length > 0 || structuredContent !== undefined ? (
        <div className="mcp-result-card">
          <strong>Tool output</strong>
          {content.map((item, index) => (
            <ContentBlock block={item} key={index} />
          ))}
          {structuredContent !== undefined ? (
            <details className="schema-details" open>
              <summary>Structured content</summary>
              <JsonBlock value={structuredContent} />
            </details>
          ) : null}
        </div>
      ) : (
        <div className="mcp-result-card">
          <strong>Response</strong>
          <JsonBlock value={result ?? props.response.body} />
        </div>
      )}

      <details className="schema-details">
        <summary>Raw JSON</summary>
        <JsonBlock value={props.response} />
      </details>
    </div>
  );
}

function ContentBlock(props: { block: unknown }) {
  if (!props.block || typeof props.block !== "object")
    return <JsonBlock value={props.block} />;
  const block = props.block as {
    type?: unknown;
    text?: unknown;
    data?: unknown;
    mimeType?: unknown;
  };
  if (block.type === "text" && typeof block.text === "string") {
    return <div className="mcp-text-output">{block.text}</div>;
  }
  if (block.type === "image" && typeof block.data === "string") {
    const mimeType =
      typeof block.mimeType === "string" ? block.mimeType : "image/png";
    return (
      <img
        className="mcp-image-output"
        src={`data:${mimeType};base64,${block.data}`}
        alt="MCP tool output"
      />
    );
  }
  if (block.type === "resource") {
    return (
      <details className="schema-details" open>
        <summary>Resource</summary>
        <JsonBlock value={props.block} />
      </details>
    );
  }
  return <JsonBlock value={props.block} />;
}

function responsePayload(body: unknown) {
  return Array.isArray(body) ? body[0] : body;
}
