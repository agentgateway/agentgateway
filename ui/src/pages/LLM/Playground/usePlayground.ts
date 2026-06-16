import { useCallback, useMemo, useRef, useState } from "react";
import { useConfig, useMCPConfig } from "../../../api";
import { extractModels } from "./extractModels";
import type { Message, PlaygroundModel } from "./types";

const API_KEY_MODE_KEY = "playgroundApiKeyMode";
const SELECTED_KEY_KEY = "playgroundSelectedKey";

function loadStoredApiKeyMode(): "none" | "managed" | "raw" {
  const v = localStorage.getItem(API_KEY_MODE_KEY);
  if (v === "managed" || v === "raw") return v;
  return "none";
}

async function fetchMcpTools(mcpBaseUrl: string): Promise<any[]> {
  const initRes = await fetch(mcpBaseUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2024-11-05",
        capabilities: {},
        clientInfo: { name: "agentgateway-playground", version: "1.0" },
      },
    }),
  });
  if (!initRes.ok) throw new Error(`MCP initialize failed: HTTP ${initRes.status}`);

  const listRes = await fetch(mcpBaseUrl, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      jsonrpc: "2.0",
      id: 2,
      method: "tools/list",
      params: {},
    }),
  });
  if (!listRes.ok) throw new Error(`MCP tools/list failed: HTTP ${listRes.status}`);
  const listData = await listRes.json();
  return listData.result?.tools ?? [];
}

function mcpToolToOpenAI(tool: any) {
  return {
    type: "function",
    function: {
      name: tool.name,
      description: tool.description ?? "",
      parameters: tool.inputSchema ?? { type: "object", properties: {} },
    },
  };
}

export function usePlayground() {
  const { data: config, isLoading } = useConfig();
  const { data: mcp } = useMCPConfig();

  const [selectedLabel, setSelectedLabel] = useState<string | null>(null);
  const [modelOverride, setModelOverride] = useState("");
  const [prompt, setPrompt] = useState("");
  const [messages, setMessages] = useState<Message[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chatEndRef = useRef<HTMLDivElement>(null);

  const [apiKeyMode, setApiKeyMode] = useState<"none" | "managed" | "raw">(loadStoredApiKeyMode);
  const [selectedApiKey, setSelectedApiKey] = useState<string>(localStorage.getItem(SELECTED_KEY_KEY) ?? "");
  const [rawApiKey, setRawApiKey] = useState<string>("");
  const [includeMcpTools, setIncludeMcpTools] = useState(false);

  const models: PlaygroundModel[] = useMemo(() =>
    config ? extractModels(config) : [],
    [config]
  );

  const selectedModel = models.find((m) => m.label === selectedLabel) ?? null;
  const effectiveModel =
    modelOverride.trim() || selectedModel?.defaultModel || selectedModel?.label || "";

  const scrollToBottom = useCallback(() => {
    setTimeout(() => chatEndRef.current?.scrollIntoView({ behavior: "smooth" }), 50);
  }, []);

  const handleSend = useCallback(async () => {
    if (!prompt.trim() || !selectedModel) return;
    setError(null);

    const userMsg: Message = { role: "user", content: prompt.trim() };
    setMessages((prev) => [...prev, userMsg]);
    setPrompt("");
    setSending(true);
    scrollToBottom();

    try {
      const allMessages = [...messages, userMsg].map((m) => ({
        role: m.role,
        content: m.content,
      }));

      const headers: Record<string, string> = { "Content-Type": "application/json" };

      // Add auth header based on key mode
      if (apiKeyMode === "managed" && selectedApiKey) {
        headers["Authorization"] = `Bearer ${selectedApiKey}`;
      } else if (apiKeyMode === "raw" && rawApiKey.trim()) {
        headers["Authorization"] = `Bearer ${rawApiKey.trim()}`;
      }

      // Fetch MCP tools if requested
      let tools: any[] | undefined;
      if (includeMcpTools && mcp?.port) {
        try {
          const mcpBaseUrl = `http://localhost:${mcp.port}/mcp`;
          const rawTools = await fetchMcpTools(mcpBaseUrl);
          tools = rawTools.map(mcpToolToOpenAI);
        } catch (e: any) {
          // Non-fatal: warn but proceed without tools
          console.warn("Failed to fetch MCP tools:", e.message);
        }
      }

      const body: Record<string, any> = {
        model: effectiveModel,
        messages: allMessages,
      };
      if (tools && tools.length > 0) body.tools = tools;

      const res = await fetch(`${selectedModel.baseUrl}/v1/chat/completions`, {
        method: "POST",
        headers,
        body: JSON.stringify(body),
      });

      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `HTTP ${res.status}`);
      }

      const data = await res.json();
      const content = data.choices?.[0]?.message?.content ?? "(empty response)";

      setMessages((prev) => [
        ...prev,
        { role: "assistant", content },
      ]);
    } catch (e: unknown) {
      const msg =
        e && typeof e === "object" && "message" in e
          ? (e as any).message
          : "Failed to get response";
      setError(msg);
      setMessages((prev) => prev.slice(0, -1));
      setPrompt(userMsg.content);
    } finally {
      setSending(false);
      scrollToBottom();
    }
  }, [prompt, selectedModel, effectiveModel, messages, scrollToBottom, apiKeyMode, selectedApiKey, rawApiKey, includeMcpTools, mcp]);

  const handleClear = useCallback(() => {
    setMessages([]);
    setPrompt("");
    setError(null);
  }, []);

  const handleSelectLabel = useCallback(
    (label: string) => {
      setSelectedLabel(label);
      const m = models.find((m) => m.label === label);
      setModelOverride(m?.defaultModel ?? "");
    },
    [models]
  );

  return {
    isLoading,
    config,
    models,
    selectedLabel,
    selectedModel,
    effectiveModel,
    modelOverride,
    prompt,
    messages,
    sending,
    error,
    chatEndRef,
    handleSend,
    handleClear,
    handleSelectLabel,
    setModelOverride,
    setPrompt,
    apiKeyMode,
    setApiKeyMode,
    selectedApiKey,
    setSelectedApiKey,
    rawApiKey,
    setRawApiKey,
    includeMcpTools,
    setIncludeMcpTools,
  };
}
