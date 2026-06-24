import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  Bot,
  Braces,
  ChevronDown,
  Clock3,
  GitBranch,
  Hash,
  KeyRound,
  Loader2,
  Send,
  TextCursorInput,
  User,
} from "lucide-react";
import { sendChatCompletion, sendMcpJsonRpc } from "../api/playgroundApi";
import { providerLabel } from "../config";
import { applyPlaygroundCors, corsNeedsUpdate, currentOrigin } from "../cors";
import { hasKeyValue, keyLabel } from "../credentialDisplay";
import { gatewayEndpoint, gatewayOrigin } from "../gatewayUrls";
import {
  useGatewayConfig,
  useStoredStringState,
  useUpdateConfig,
} from "../hooks";
import { CatalogModelSelector } from "../components/CatalogModelSelector";
import {
  Dropdown,
  Field,
  FieldGroup,
  JsonBlock,
  PageHeader,
  Panel,
  StatusBanner,
} from "../components/Primitives";
import { ProviderIcon } from "../components/ProviderIcon";
import {
  extractMcpTools,
  initializeMcpSession,
  nextRpcId,
  type McpTool,
} from "../mcp";
import {
  isWildcardModelName,
  modelProviderLabel,
  resolveModelName,
  wildcardModelPrefix,
  wildcardResolvedSuffix,
} from "../modelResolution";
import type { LlmModel, LlmProvider, ProviderName } from "../types";

type ChatMessage = {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  tool_call_id?: string;
  name?: string;
  tool_calls?: ToolCall[];
  arguments?: unknown;
  meta?: MessageMeta;
  raw?: unknown;
};

type MessageMeta = {
  status?: string;
  model?: string;
  provider?: string;
  latencyMs?: number;
  inputTokens?: number;
  outputTokens?: number;
  totalTokens?: number;
  cost?: number;
};

type PlaygroundTool = McpTool & {
  functionName: string;
};

type ToolCall = {
  id: string;
  type?: string;
  function?: {
    name?: string;
    arguments?: string;
  };
};

type ToolExecution = {
  call: ToolCall;
  tool?: PlaygroundTool;
  arguments: unknown;
  result: unknown;
};

type RunStep = {
  label: string;
  state: "pending" | "active" | "done" | "error";
};

type RequestModelOption =
  | { kind: "model"; config: LlmModel }
  | { kind: "virtual" };

const storageKeys = {
  model: "playgroundModel",
  specificModel: "playgroundSpecificModel",
  apiKeyMode: "playgroundApiKeyMode",
  selectedKey: "playgroundSelectedKeyRef",
};

const legacySecretStorageKeys = [
  "playgroundSelectedKey",
  "playgroundSystemMessage",
  "playgroundPrompt",
  "playgroundMcpEnabled",
];

export function PlaygroundPage() {
  const config = useGatewayConfig();
  const updateConfig = useUpdateConfig();
  const models = useMemo(() => config.data?.llm?.models ?? [], [config.data]);
  const virtualModels = useMemo(
    () => config.data?.llm?.virtualModels ?? [],
    [config.data],
  );
  const providers = useMemo(
    () => config.data?.llm?.providers ?? [],
    [config.data],
  );
  const modelOptions = useMemo(
    () => [
      ...models.map((item) => ({
        kind: "model" as const,
        name: item.name,
        icon: (
          <ProviderIcon
            provider={modelProviderLabel(item, providers) as ProviderName}
          />
        ),
        searchText: `${item.name} ${modelProviderLabel(item, providers)} ${providerLabel(item.provider)}`,
        config: item,
      })),
      ...virtualModels.map((item) => ({
        kind: "virtual" as const,
        name: item.name,
        icon: <GitBranch size={16} />,
        searchText: `${item.name} virtual`,
        config: item,
      })),
    ],
    [models, providers, virtualModels],
  );
  const virtualKeys = useMemo(
    () => config.data?.llm?.policies?.apiKey?.keys ?? [],
    [config.data],
  );
  const rawVirtualKeys = useMemo(
    () => virtualKeys.filter(hasKeyValue),
    [virtualKeys],
  );
  const [storedModel, setStoredModel] = useStoredStringState(
    storageKeys.model,
    "",
  );
  const [model, setModel] = useState(() => queryModel() ?? storedModel);
  const llmBaseUrl = gatewayOrigin(config.data?.llm?.port ?? 4000);
  const [specificModel, setSpecificModel] = useStoredStringState(
    storageKeys.specificModel,
    "",
  );
  const [apiKeyMode, setApiKeyMode] = useState<"saved" | "raw">(() =>
    storedApiKeyMode(),
  );
  const [selectedKeyRef, setSelectedKeyRef] = useStoredStringState(
    storageKeys.selectedKey,
    "",
  );
  const [apiKey, setApiKey] = useState("");
  const [system, setSystem] = useState("You are a concise assistant.");
  const [systemOpen, setSystemOpen] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [runSteps, setRunSteps] = useState<RunStep[]>([]);
  const derivedMcpBaseUrl = gatewayEndpoint(
    config.data?.mcp?.port ?? 3000,
    "/mcp",
  );
  const [mcpEnabled, setMcpEnabled] = useState(false);
  const [mcpSessionId, setMcpSessionId] = useState("");
  const [mcpTools, setMcpTools] = useState<PlaygroundTool[]>([]);
  const mcpServerCount = config.data?.mcp?.targets?.length ?? 0;

  useEffect(() => {
    for (const key of legacySecretStorageKeys) localStorage.removeItem(key);
  }, []);

  const savedModelExists = modelOptions.some((item) => item.name === model);
  const selectedModel =
    (savedModelExists ? model : "") || modelOptions[0]?.name || "";
  const selectedModelOption = modelOptions.find(
    (item) => item.name === selectedModel,
  );
  const selectedModelConfig =
    selectedModelOption?.kind === "model"
      ? selectedModelOption.config
      : undefined;
  const displayRequestModel = playgroundRequestModel(
    selectedModelOption,
    selectedModel,
    specificModel,
    providers,
  );
  const wildcardPrefix =
    selectedModelConfig && isWildcardModelName(selectedModelConfig.name)
      ? wildcardModelPrefix(selectedModelConfig.name)
      : "";
  const specificModelSuffix =
    selectedModelConfig && isWildcardModelName(selectedModelConfig.name)
      ? wildcardResolvedSuffix(
          displayRequestModel,
          selectedModelConfig.name,
          wildcardPrefix,
        )
      : specificModel;
  const selectedCatalogProvider = selectedModelConfig
    ? modelProviderLabel(selectedModelConfig, providers)
    : null;
  const selectedKeyOptionRef = rawVirtualKeys.some(
    (item, index) => virtualKeyStorageRef(item, index) === selectedKeyRef,
  )
    ? selectedKeyRef
    : rawVirtualKeys[0]
      ? virtualKeyStorageRef(rawVirtualKeys[0], 0)
      : "";
  const savedKey =
    rawVirtualKeys.find(
      (item, index) =>
        virtualKeyStorageRef(item, index) === selectedKeyOptionRef,
    )?.key ??
    rawVirtualKeys[0]?.key ??
    "";
  const selectedKeyValue =
    apiKeyMode === "saved" && rawVirtualKeys.length > 0 ? savedKey : apiKey;
  const needsCors = config.data
    ? corsNeedsUpdate(config.data.llm?.policies?.cors, "llm")
    : false;
  const needsMcpCors =
    mcpEnabled && config.data
      ? corsNeedsUpdate(config.data.mcp?.policies?.cors, "mcp")
      : false;
  const sendBlockers = sendReadinessBlockers({
    loading,
    displayRequestModel,
    prompt,
    modelOptionsCount: modelOptions.length,
    selectedKeyValue,
    apiKeyMode,
    virtualKeysCount: rawVirtualKeys.length,
  });

  useEffect(() => {
    if (modelOptions.length > 0 && model && !savedModelExists) setModel("");
  }, [model, modelOptions, savedModelExists]);

  useEffect(() => {
    setStoredModel(model);
  }, [model, setStoredModel]);

  useEffect(() => {
    if (
      rawVirtualKeys[0]?.key &&
      !rawVirtualKeys.some(
        (item, index) => virtualKeyStorageRef(item, index) === selectedKeyRef,
      )
    ) {
      setSelectedKeyRef(virtualKeyStorageRef(rawVirtualKeys[0], 0));
    }
  }, [selectedKeyRef, setSelectedKeyRef, rawVirtualKeys]);

  useEffect(() => {
    localStorage.setItem(storageKeys.apiKeyMode, apiKeyMode);
  }, [apiKeyMode]);

  useEffect(() => {
    if (mcpServerCount === 0 && mcpEnabled) setMcpEnabled(false);
  }, [mcpEnabled, mcpServerCount]);

  async function loadMcpTools() {
    let sessionId = await initializeMcpSession(
      derivedMcpBaseUrl,
      "agentgateway-ui-llm-playground",
      mcpSessionId,
    );
    setMcpSessionId(sessionId);
    const response = await sendMcpJsonRpc({
      baseUrl: derivedMcpBaseUrl,
      sessionId,
      body: {
        jsonrpc: "2.0",
        id: nextRpcId(),
        method: "tools/list",
        params: {},
      },
    });
    if (response.sessionId) {
      sessionId = response.sessionId;
      setMcpSessionId(response.sessionId);
    }
    const tools = extractMcpTools(response.body).map((tool) => ({
      ...tool,
      functionName: toFunctionName(tool.name),
    }));
    setMcpTools(tools);
    return { tools, sessionId };
  }

  async function send() {
    const userMessage = prompt.trim();
    if (!userMessage) return;
    const userChatMessage: ChatMessage = { role: "user", content: userMessage };
    const outboundMessages: ChatMessage[] = [
      ...(system.trim()
        ? [{ role: "system" as const, content: system.trim() }]
        : []),
      ...messages,
      userChatMessage,
    ];
    setLoading(true);
    setError(null);
    setRunSteps([
      { label: "Preparing request", state: "active" },
      { label: "Sending chat completion", state: "pending" },
      { label: "Waiting for model response", state: "pending" },
    ]);
    try {
      const requestModel = playgroundRequestModel(
        selectedModelOption,
        selectedModel,
        specificModel,
        providers,
      );
      let availableMcpTools = mcpTools;
      let activeMcpSessionId = mcpSessionId;
      if (mcpEnabled && mcpTools.length === 0) {
        setRunSteps([
          { label: "Preparing request", state: "done" },
          { label: "Initializing MCP tools", state: "active" },
          { label: "Sending chat completion", state: "pending" },
          { label: "Waiting for model response", state: "pending" },
        ]);
        const loaded = await loadMcpTools();
        availableMcpTools = loaded.tools;
        activeMcpSessionId = loaded.sessionId;
        if (availableMcpTools.length === 0) {
          throw new Error("No MCP tools are available from the MCP gateway.");
        }
      }
      const tools =
        mcpEnabled && availableMcpTools.length > 0
          ? availableMcpTools.map(toolToOpenAiFunction)
          : undefined;
      setRunSteps([
        { label: "Preparing request", state: "done" },
        ...(mcpEnabled
          ? [{ label: "Initializing MCP tools", state: "done" as const }]
          : []),
        { label: "Sending chat completion", state: "active" },
        { label: "Waiting for model response", state: "pending" },
      ]);
      const started = performance.now();
      const response = await sendChatCompletion({
        baseUrl: llmBaseUrl,
        model: requestModel,
        apiKey: selectedKeyValue,
        messages: toOpenAiMessages(outboundMessages),
        tools,
      });
      const firstLatencyMs = Math.round(performance.now() - started);
      setRunSteps([
        { label: "Preparing request", state: "done" },
        ...(mcpEnabled
          ? [{ label: "Initializing MCP tools", state: "done" as const }]
          : []),
        { label: "Sending chat completion", state: "done" },
        { label: "Waiting for model response", state: "done" },
      ]);
      const toolCalls = extractToolCalls(response);
      if (mcpEnabled && toolCalls.length > 0) {
        const assistantMessage: ChatMessage = {
          role: "assistant",
          content: assistantContent(response),
          tool_calls: toolCalls,
          meta: responseMeta(
            response,
            requestModel,
            selectedCatalogProvider ?? undefined,
            firstLatencyMs,
          ),
          raw: response,
        };
        setMessages((current) => [
          ...current,
          userChatMessage,
          assistantMessage,
        ]);
        setPrompt("");
        setRunSteps([
          { label: "Preparing request", state: "done" },
          { label: "Initializing MCP tools", state: "done" },
          { label: "Sending chat completion", state: "done" },
          {
            label: `Calling ${toolCalls.length} MCP ${toolCalls.length === 1 ? "tool" : "tools"}`,
            state: "active",
          },
          { label: "Sending tool results", state: "pending" },
          { label: "Waiting for final response", state: "pending" },
        ]);
        const executions = await executeToolCalls(
          toolCalls,
          availableMcpTools,
          derivedMcpBaseUrl,
          activeMcpSessionId,
          setMcpSessionId,
        );
        const toolMessages: ChatMessage[] = executions.map((execution) => ({
          role: "tool",
          tool_call_id: execution.call.id,
          name: execution.tool?.functionName ?? execution.call.function?.name,
          content: toolResultText(execution.result),
          arguments: execution.arguments,
          raw: execution.result,
        }));
        setMessages((current) => [...current, ...toolMessages]);
        const followUpMessages = [
          ...outboundMessages,
          assistantMessage,
          ...toolMessages,
        ];
        setRunSteps([
          { label: "Preparing request", state: "done" },
          { label: "Initializing MCP tools", state: "done" },
          { label: "Sending chat completion", state: "done" },
          {
            label: `Calling ${toolCalls.length} MCP ${toolCalls.length === 1 ? "tool" : "tools"}`,
            state: "done",
          },
          { label: "Sending tool results", state: "active" },
          { label: "Waiting for final response", state: "pending" },
        ]);
        const finalStarted = performance.now();
        const finalResponse = await sendChatCompletion({
          baseUrl: llmBaseUrl,
          model: requestModel,
          apiKey: selectedKeyValue,
          messages: toOpenAiMessages(followUpMessages),
          tools,
        });
        const finalLatencyMs = Math.round(performance.now() - finalStarted);
        setRunSteps([
          { label: "Preparing request", state: "done" },
          { label: "Initializing MCP tools", state: "done" },
          { label: "Sending chat completion", state: "done" },
          {
            label: `Calling ${toolCalls.length} MCP ${toolCalls.length === 1 ? "tool" : "tools"}`,
            state: "done",
          },
          { label: "Sending tool results", state: "done" },
          { label: "Waiting for final response", state: "done" },
        ]);
        setMessages((current) => [
          ...current,
          {
            role: "assistant",
            content: assistantText(finalResponse),
            meta: responseMeta(
              finalResponse,
              requestModel,
              selectedCatalogProvider ?? undefined,
              finalLatencyMs,
            ),
            raw: finalResponse,
          },
        ]);
      } else {
        setMessages((current) => [
          ...current,
          userChatMessage,
          {
            role: "assistant",
            content: assistantText(response),
            meta: responseMeta(
              response,
              requestModel,
              selectedCatalogProvider ?? undefined,
              firstLatencyMs,
            ),
            raw: response,
          },
        ]);
        setPrompt("");
      }
    } catch (err) {
      setRunSteps((current) =>
        current.map((step) =>
          step.state === "active" ? { ...step, state: "error" } : step,
        ),
      );
      setError(err instanceof Error ? err.message : "Request failed");
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="page-stack">
      <PageHeader
        title="LLM Playground"
        description="Send a real chat completion request through the configured gateway for setup debugging."
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
                updateConfig.mutate((next) => applyPlaygroundCors(next, "llm"))
              }
            >
              Apply CORS
            </button>
          }
        >
          Add {currentOrigin()} to the LLM CORS policy so this playground can
          call the gateway from the browser.
        </StatusBanner>
      ) : null}
      {needsMcpCors ? (
        <StatusBanner
          state="warn"
          title="MCP browser access is not allowed"
          action={
            <button
              className="button"
              type="button"
              disabled={updateConfig.isPending}
              onClick={() =>
                updateConfig.mutate((next) => applyPlaygroundCors(next, "mcp"))
              }
            >
              Apply MCP CORS
            </button>
          }
        >
          Add {currentOrigin()} to the MCP CORS policy so the playground can
          list and call MCP tools from the browser.
        </StatusBanner>
      ) : null}
      {modelOptions.length === 0 ? (
        <StatusBanner state="warn" title="No configured models">
          Create a model before testing chat traffic.
        </StatusBanner>
      ) : null}
      {error ? (
        <StatusBanner state="bad" title="Playground request failed">
          {error}
        </StatusBanner>
      ) : null}
      <section className="playground-shell">
        <Panel className="playground-config-panel">
          <div className="section-heading">
            <h3>Configuration</h3>
          </div>
          <div className="playground-setup-bar">
            <div className="playground-control-row">
              <FieldGroup label="Model">
                <Dropdown
                  ariaLabel="Model"
                  value={selectedModel}
                  placeholder="No models"
                  searchable
                  options={modelOptions.map((item) => ({
                    value: item.name,
                    label:
                      item.kind === "virtual" ? (
                        <span className="select-option-copy">
                          <strong>{item.name}</strong>
                          <small>Virtual model</small>
                        </span>
                      ) : (
                        item.name
                      ),
                    icon: item.icon,
                    searchText: item.searchText,
                  }))}
                  onChange={setModel}
                />
              </FieldGroup>
              {selectedModelConfig &&
              isWildcardModelName(selectedModelConfig.name) ? (
                <Field
                  label="Specific model"
                  hint="Model uses a wildcard; specify the specific model."
                >
                  <div className="target-resolved-composite">
                    {wildcardPrefix ? (
                      <span className="target-prefix">{wildcardPrefix}</span>
                    ) : null}
                    <CatalogModelSelector
                      ariaLabel="Specific model"
                      value={specificModelSuffix}
                      provider={selectedCatalogProvider}
                      onChange={(value) =>
                        setSpecificModel(`${wildcardPrefix}${value}`)
                      }
                    />
                  </div>
                </Field>
              ) : (
                <div aria-hidden="true" />
              )}
            </div>
            <div className="playground-control-row">
              <FieldGroup label="Virtual API key">
                <Dropdown
                  ariaLabel="Virtual API key"
                  value={
                    apiKeyMode === "saved" && rawVirtualKeys.length > 0
                      ? selectedKeyOptionRef
                      : "__raw__"
                  }
                  options={[
                    ...rawVirtualKeys.map((item, index) => ({
                      value: virtualKeyStorageRef(item, index),
                      label: keyLabel(item),
                      icon: <KeyRound size={16} />,
                    })),
                    {
                      value: "__raw__",
                      label: "Raw value",
                      icon: <TextCursorInput size={16} />,
                    },
                  ]}
                  onChange={(next) => {
                    if (next === "__raw__") {
                      setApiKeyMode("raw");
                    } else {
                      setApiKeyMode("saved");
                      setSelectedKeyRef(next);
                    }
                  }}
                />
              </FieldGroup>
              {apiKeyMode === "raw" || rawVirtualKeys.length === 0 ? (
                <Field label="Raw API key">
                  <input
                    value={apiKey}
                    type="text"
                    className="masked-secret-input"
                    autoComplete="off"
                    autoCorrect="off"
                    autoCapitalize="none"
                    data-1p-ignore="true"
                    data-lpignore="true"
                    data-form-type="other"
                    name="agw-playground-raw-api-key"
                    spellCheck={false}
                    onChange={(event) => setApiKey(event.target.value)}
                    placeholder="Optional Bearer token"
                  />
                </Field>
              ) : (
                <div aria-hidden="true" />
              )}
            </div>
            {mcpServerCount > 0 ? (
              <div className="playground-tool-cell">
                <label className="config-option-row">
                  <input
                    type="checkbox"
                    checked={mcpEnabled}
                    onChange={(event) => setMcpEnabled(event.target.checked)}
                  />
                  <span>
                    <strong>
                      Include MCP tools ({mcpServerCount}{" "}
                      {mcpServerCount === 1 ? "server" : "servers"})
                    </strong>
                    <small>
                      Let the model call tools exposed by the MCP gateway.
                    </small>
                  </span>
                </label>
              </div>
            ) : null}
          </div>
          <details
            className="system-prompt-details"
            open={systemOpen}
            onToggle={(event) => setSystemOpen(event.currentTarget.open)}
          >
            <summary>
              <span>System prompt</span>
              <small>
                {system.trim()
                  ? truncateOneLine(system, 96)
                  : "No system prompt"}
              </small>
              <ChevronDown size={16} />
            </summary>
            <Field label="System prompt">
              <textarea
                rows={4}
                value={system}
                onChange={(event) => setSystem(event.target.value)}
              />
            </Field>
          </details>
        </Panel>
        <Panel className="playground-chat-panel">
          {runSteps.length ? <RunTimeline steps={runSteps} /> : null}
          <div className="mini-chat">
            {messages.length === 0 && !loading ? (
              <div className="chat-empty">No messages yet.</div>
            ) : (
              messages.map((message, index) => (
                <ChatMessageView
                  message={message}
                  key={`${message.role}-${index}`}
                />
              ))
            )}
            {loading ? (
              <div className="chat-message user pending">
                <div className="chat-avatar">
                  <User size={16} />
                </div>
                <div className="chat-bubble">
                  <Loader2 className="spin" size={15} />
                  Sending
                </div>
              </div>
            ) : null}
          </div>
          <Field label="User message">
            <textarea
              rows={6}
              value={prompt}
              onChange={(event) => setPrompt(event.target.value)}
              onKeyDown={(event) => {
                if (
                  event.key === "Enter" &&
                  !event.shiftKey &&
                  !event.nativeEvent.isComposing
                ) {
                  event.preventDefault();
                  if (sendBlockers.length === 0) void send();
                }
              }}
              placeholder="Ask a test question..."
            />
          </Field>
          <div className="button-row">
            <button
              className="button primary"
              type="button"
              disabled={sendBlockers.length > 0}
              onClick={send}
            >
              <Send size={16} />
              Send
            </button>
            <button
              className="button"
              type="button"
              disabled={loading || messages.length === 0}
              onClick={() => {
                setMessages([]);
                setRunSteps([]);
                setPrompt("");
              }}
            >
              Clear
            </button>
          </div>
          {sendBlockers.length ? (
            <div className="playground-send-blockers">
              {sendBlockers.map((blocker) => (
                <span key={blocker}>{blocker}</span>
              ))}
            </div>
          ) : null}
        </Panel>
      </section>
    </div>
  );
}

function storedApiKeyMode(): "saved" | "raw" {
  return localStorage.getItem(storageKeys.apiKeyMode) === "raw"
    ? "raw"
    : "saved";
}

function virtualKeyStorageRef(key: { metadata?: unknown }, index: number) {
  const metadata =
    key.metadata &&
    typeof key.metadata === "object" &&
    !Array.isArray(key.metadata)
      ? (key.metadata as Record<string, unknown>)
      : {};
  const name =
    typeof metadata.name === "string" && metadata.name.trim()
      ? metadata.name.trim()
      : "";
  return name ? `name:${name}` : `index:${index}`;
}

function queryModel() {
  return new URLSearchParams(window.location.search).get("model");
}

function playgroundRequestModel(
  option: RequestModelOption | undefined,
  selectedModel: string,
  specificModel: string,
  providers: LlmProvider[],
) {
  if (option?.kind === "virtual") return selectedModel;
  if (option?.kind !== "model")
    return resolveModelName(undefined, specificModel, providers);
  const normalized = normalizedSpecificModel(option.config, specificModel);
  if (isWildcardModelName(option.config.name) && !normalized) return "";
  return resolveModelName(option.config, normalized, providers);
}

function normalizedSpecificModel(model: LlmModel, specificModel: string) {
  if (!isWildcardModelName(model.name)) return specificModel;
  const trimmed = specificModel.trim();
  const prefix = wildcardModelPrefix(model.name);
  if (!trimmed || trimmed === prefix) return "";
  if (prefix && !trimmed.startsWith(prefix)) return "";
  return trimmed;
}

function ChatMessageView(props: { message: ChatMessage }) {
  const message = props.message;
  const Icon =
    message.role === "assistant"
      ? Bot
      : message.role === "tool"
        ? Braces
        : User;
  const inspectValue = message.raw
    ? { message: toOpenAiMessages([message])[0], raw: message.raw }
    : { message: toOpenAiMessages([message])[0] };
  return (
    <div className={`chat-message ${message.role}`}>
      <div className="chat-avatar">
        <Icon size={16} />
      </div>
      <div className="chat-bubble">
        {message.role === "assistant" && message.tool_calls?.length ? (
          <ToolCallSummary message={message} />
        ) : message.role === "tool" ? (
          <ToolResultSummary message={message} />
        ) : (
          message.content
        )}
        {message.meta ? <MessageMetaChips meta={message.meta} /> : null}
        <details className="message-inspector">
          <summary>Inspect</summary>
          <JsonBlock value={inspectValue} />
        </details>
      </div>
    </div>
  );
}

function MessageMetaChips(props: { meta: MessageMeta }) {
  const maybeChips: Array<{
    key: string;
    label: string;
    icon: ReactNode;
  } | null> = [
    props.meta.provider
      ? { key: "provider", label: props.meta.provider, icon: <Bot size={13} /> }
      : null,
    props.meta.status
      ? { key: "status", label: props.meta.status, icon: <Braces size={13} /> }
      : null,
    props.meta.model
      ? { key: "model", label: props.meta.model, icon: <GitBranch size={13} /> }
      : null,
    props.meta.latencyMs !== undefined
      ? {
          key: "latency",
          label: `${formatLatency(props.meta.latencyMs)}`,
          icon: <Clock3 size={13} />,
        }
      : null,
    props.meta.totalTokens !== undefined
      ? {
          key: "tokens",
          label: formatTokenMeta(props.meta),
          icon: <Hash size={13} />,
        }
      : null,
    props.meta.cost !== undefined
      ? {
          key: "cost",
          label: formatCost(props.meta.cost),
          icon: <Braces size={13} />,
        }
      : null,
  ];
  const chips = maybeChips.filter(
    (item): item is { key: string; label: string; icon: ReactNode } =>
      Boolean(item),
  );
  if (!chips.length) return null;
  return (
    <div className="message-meta-chips">
      {chips.map((chip) => (
        <span className="message-meta-chip" key={chip.key}>
          {chip.icon}
          {chip.label}
        </span>
      ))}
    </div>
  );
}

function RunTimeline(props: { steps: RunStep[] }) {
  return (
    <div className="playground-run-timeline" aria-label="Request progress">
      {props.steps.map((step, index) => (
        <div
          className={`run-step ${step.state}`}
          key={`${index}-${step.label}`}
        >
          <span className="run-step-dot">
            {step.state === "active" ? (
              <Loader2 className="spin" size={12} />
            ) : null}
          </span>
          <span>{step.label}</span>
        </div>
      ))}
    </div>
  );
}

function ToolCallSummary(props: { message: ChatMessage }) {
  return (
    <div className="tool-call-summary">
      {props.message.content.trim() ? <p>{props.message.content}</p> : null}
      {props.message.tool_calls?.map((call) => (
        <div className="tool-call-row" key={call.id}>
          <span className="tool-pill">Tool call</span>
          <strong>{call.function?.name || "unknown"}</strong>
          <small>
            {summarizeValue(parseToolArguments(call.function?.arguments))}
          </small>
        </div>
      ))}
    </div>
  );
}

function ToolResultSummary(props: { message: ChatMessage }) {
  return (
    <div className="tool-call-summary">
      <div className="tool-call-row">
        <span className="tool-pill">Tool result</span>
        <strong>{props.message.name || "unknown"}</strong>
        <small>{summarizeToolResult(props.message.content)}</small>
      </div>
    </div>
  );
}

function assistantText(response: unknown) {
  return assistantContent(response) || JSON.stringify(response, null, 2);
}

function assistantContent(response: unknown) {
  if (response && typeof response === "object") {
    const choices = (response as { choices?: unknown }).choices;
    if (Array.isArray(choices)) {
      const first = choices[0] as
        | {
            message?: { content?: unknown };
            delta?: { content?: unknown };
            text?: unknown;
          }
        | undefined;
      const content =
        first?.message?.content ?? first?.delta?.content ?? first?.text;
      if (typeof content === "string" && content.trim()) return content;
    }
  }
  return "";
}

function responseMeta(
  response: unknown,
  model: string,
  provider: string | undefined,
  latencyMs: number,
): MessageMeta {
  const usage = extractUsage(response);
  const cost = extractCost(response);
  return {
    status: "OK",
    model,
    provider,
    latencyMs,
    inputTokens: usage.inputTokens,
    outputTokens: usage.outputTokens,
    totalTokens: usage.totalTokens,
    cost,
  };
}

function extractUsage(response: unknown) {
  if (!response || typeof response !== "object") return {};
  const usage = (response as { usage?: unknown }).usage;
  if (!usage || typeof usage !== "object" || Array.isArray(usage)) return {};
  const record = usage as Record<string, unknown>;
  const inputTokens = numberValue(
    record.prompt_tokens ?? record.input_tokens ?? record.inputTokens,
  );
  const outputTokens = numberValue(
    record.completion_tokens ?? record.output_tokens ?? record.outputTokens,
  );
  const totalTokens =
    numberValue(record.total_tokens ?? record.totalTokens) ??
    addOptional(inputTokens, outputTokens);
  return { inputTokens, outputTokens, totalTokens };
}

function extractCost(response: unknown) {
  if (!response || typeof response !== "object") return undefined;
  const direct = numberValue((response as { cost?: unknown }).cost);
  if (direct !== undefined) return direct;
  const usage = (response as { usage?: unknown }).usage;
  if (usage && typeof usage === "object" && !Array.isArray(usage)) {
    return numberValue((usage as Record<string, unknown>).cost);
  }
  return undefined;
}

function toolToOpenAiFunction(tool: PlaygroundTool) {
  return {
    type: "function",
    function: {
      name: tool.functionName,
      description: tool.description || `MCP tool ${tool.name}`,
      parameters: normalizeInputSchema(tool.inputSchema),
    },
  };
}

function normalizeInputSchema(schema: unknown) {
  if (schema && typeof schema === "object" && !Array.isArray(schema))
    return schema;
  return { type: "object", properties: {} };
}

function toFunctionName(name: string) {
  const normalized = name.replace(/[^a-zA-Z0-9_-]/g, "_").slice(0, 64);
  return normalized || "mcp_tool";
}

function extractToolCalls(response: unknown): ToolCall[] {
  if (!response || typeof response !== "object") return [];
  const choices = (response as { choices?: unknown }).choices;
  if (!Array.isArray(choices)) return [];
  const first = choices[0] as
    | { message?: { tool_calls?: unknown } }
    | undefined;
  const calls = first?.message?.tool_calls;
  if (!Array.isArray(calls)) return [];
  return calls.filter((call): call is ToolCall =>
    Boolean(
      call &&
      typeof call === "object" &&
      typeof (call as { id?: unknown }).id === "string",
    ),
  );
}

function toOpenAiMessages(messages: ChatMessage[]) {
  return messages.map((message) => {
    if (message.role === "assistant") {
      return {
        role: message.role,
        content: message.content,
        ...(message.tool_calls?.length
          ? { tool_calls: message.tool_calls }
          : {}),
      };
    }
    if (message.role === "tool") {
      return {
        role: message.role,
        tool_call_id: message.tool_call_id,
        name: message.name,
        content: message.content,
      };
    }
    return {
      role: message.role,
      content: message.content,
    };
  });
}

function sendReadinessBlockers(args: {
  loading: boolean;
  displayRequestModel: string;
  prompt: string;
  modelOptionsCount: number;
  selectedKeyValue: string;
  apiKeyMode: "saved" | "raw";
  virtualKeysCount: number;
}) {
  const blockers: string[] = [];
  if (args.loading) blockers.push("Request in progress");
  if (args.modelOptionsCount === 0) blockers.push("Configure a model first");
  if (!args.displayRequestModel) blockers.push("Select a concrete model");
  return blockers;
}

async function executeToolCalls(
  calls: ToolCall[],
  tools: PlaygroundTool[],
  baseUrl: string,
  sessionId: string,
  setSessionId: (value: string) => void,
) {
  const executions: ToolExecution[] = [];
  let activeSessionId =
    sessionId ||
    (await initializeMcpSession(
      baseUrl,
      "agentgateway-ui-llm-playground",
      sessionId,
    ));
  setSessionId(activeSessionId);
  for (const call of calls) {
    const functionName = call.function?.name ?? "";
    const tool = tools.find(
      (candidate) =>
        candidate.functionName === functionName ||
        candidate.name === functionName,
    );
    const args = parseToolArguments(call.function?.arguments);
    const response = await sendMcpJsonRpc({
      baseUrl,
      sessionId: activeSessionId,
      body: {
        jsonrpc: "2.0",
        id: nextRpcId(),
        method: "tools/call",
        params: {
          name: tool?.name ?? functionName,
          arguments: args,
        },
      },
    });
    if (response.sessionId) {
      activeSessionId = response.sessionId;
      setSessionId(response.sessionId);
    }
    executions.push({ call, tool, arguments: args, result: response.body });
  }
  return executions;
}

function parseToolArguments(value: string | undefined) {
  if (!value?.trim()) return {};
  const parsed = JSON.parse(value) as unknown;
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
  return parsed;
}

function toolResultText(result: unknown) {
  const payload =
    result && typeof result === "object"
      ? ((result as { result?: unknown }).result ?? result)
      : result;
  return typeof payload === "string" ? payload : JSON.stringify(payload);
}

function summarizeToolResult(value: string) {
  try {
    const parsed = JSON.parse(value) as unknown;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      const content = (parsed as { content?: unknown }).content;
      if (Array.isArray(content)) {
        const text = content
          .map((item) =>
            item && typeof item === "object"
              ? (item as { text?: unknown }).text
              : undefined,
          )
          .find(
            (item): item is string =>
              typeof item === "string" && item.trim().length > 0,
          );
        if (text) return truncateOneLine(text, 160);
      }
    }
    return summarizeValue(parsed);
  } catch {
    return truncateOneLine(value, 160);
  }
}

function summarizeValue(value: unknown) {
  if (
    !value ||
    (typeof value === "object" &&
      !Array.isArray(value) &&
      Object.keys(value).length === 0)
  ) {
    return "no arguments";
  }
  return truncateOneLine(
    typeof value === "string" ? value : JSON.stringify(value),
    160,
  );
}

function numberValue(value: unknown) {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function addOptional(first: number | undefined, second: number | undefined) {
  if (first === undefined && second === undefined) return undefined;
  return (first ?? 0) + (second ?? 0);
}

function formatLatency(value: number) {
  return value >= 1000
    ? `${(value / 1000).toFixed(value >= 10_000 ? 0 : 1)}s`
    : `${value}ms`;
}

function formatNumber(value: number) {
  return new Intl.NumberFormat().format(value);
}

function formatTokenMeta(meta: MessageMeta) {
  if (meta.inputTokens !== undefined || meta.outputTokens !== undefined) {
    return `${formatNumber(meta.inputTokens ?? 0)} in · ${formatNumber(meta.outputTokens ?? 0)} out`;
  }
  return `${formatNumber(meta.totalTokens ?? 0)} tokens`;
}

function formatCost(value: number) {
  return new Intl.NumberFormat(undefined, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

function truncateOneLine(value: string, max: number) {
  const line = value.replace(/\s+/g, " ").trim();
  return line.length > max ? `${line.slice(0, max - 1)}...` : line;
}
