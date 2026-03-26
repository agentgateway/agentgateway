import styled from "@emotion/styled";
import { Button, Card, Input, Select, Spin, Tag, Typography } from "antd";
import { Brain, Send, Trash2 } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import { useConfig } from "../../api";

const { TextArea } = Input;
const { Text } = Typography;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface PlaygroundModel {
  /** Display label (the llm.models[].name pattern, e.g. "*") */
  label: string;
  /** Default model name to send in the request (params.model or empty) */
  defaultModel: string;
  /** Provider label (openAI, gemini, etc.) */
  provider: string;
  /** The base URL to send chat completions to (the gateway endpoint) */
  baseUrl: string;
}

interface Message {
  role: "user" | "assistant";
  content: string;
}

// ---------------------------------------------------------------------------
// Styled components
// ---------------------------------------------------------------------------

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
`;

const PageTitle = styled.h1`
  margin: 0 0 4px;
  font-size: 24px;
  font-weight: 600;
`;

const PageSubtitle = styled.p`
  margin: 0;
  color: var(--color-text-secondary);
  font-size: 14px;
`;

const PlaygroundLayout = styled.div`
  display: grid;
  grid-template-columns: 280px 1fr;
  gap: var(--spacing-lg);
  align-items: start;

  @media (max-width: 768px) {
    grid-template-columns: 1fr;
  }
`;

const SidebarSection = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
`;

const SectionLabel = styled.div`
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-secondary);
  margin-bottom: 6px;
  text-transform: uppercase;
  letter-spacing: 0.05em;
`;

const ChatContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
  min-height: 400px;
  flex: 1;
  overflow-y: auto;
`;

const MessageBubble = styled.div<{ role: "user" | "assistant" }>`
  padding: 12px 16px;
  border-radius: 12px;
  max-width: 85%;
  font-size: 14px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
  align-self: ${({ role }) => (role === "user" ? "flex-end" : "flex-start")};
  background: ${({ role }) =>
    role === "user" ? "var(--color-primary)" : "var(--color-bg-hover)"};
  color: ${({ role }) =>
    role === "user" ? "#fff" : "var(--color-text-base)"};
`;

const InputRow = styled.div`
  display: flex;
  gap: 8px;
  margin-top: auto;
`;

const MarkdownContent = styled.div`
  /* Reset margins on first/last child to avoid extra spacing */
  > *:first-child { margin-top: 0; }
  > *:last-child { margin-bottom: 0; }

  p { margin: 0 0 8px; }
  p:last-child { margin-bottom: 0; }

  code {
    background: rgba(0, 0, 0, 0.12);
    border-radius: 3px;
    padding: 1px 5px;
    font-size: 13px;
    font-family: var(--font-family-code);
  }

  pre {
    background: rgba(0, 0, 0, 0.15);
    border-radius: 6px;
    padding: 10px 14px;
    overflow-x: auto;
    margin: 6px 0;
    code {
      background: none;
      padding: 0;
    }
  }

  ul, ol {
    margin: 4px 0;
    padding-left: 20px;
  }

  li { margin: 2px 0; }

  h1, h2, h3, h4 {
    margin: 8px 0 4px;
    font-weight: 600;
  }

  blockquote {
    margin: 4px 0;
    padding-left: 10px;
    border-left: 3px solid rgba(255, 255, 255, 0.4);
    opacity: 0.85;
  }

  table {
    border-collapse: collapse;
    margin: 6px 0;
    th, td {
      border: 1px solid rgba(0, 0, 0, 0.2);
      padding: 4px 8px;
    }
  }
`;

const EmptyState = styled.div`
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-tertiary);
  font-size: 14;
`;

const EndpointInfo = styled.div`
  font-size: 12px;
  color: var(--color-text-secondary);
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-sm);
  padding: 6px 10px;
  font-family: var(--font-family-code);
  word-break: break-all;
`;

// ---------------------------------------------------------------------------
// Provider colors
// ---------------------------------------------------------------------------

const PROVIDER_COLORS: Record<string, string> = {
  openAI: "blue",
  gemini: "cyan",
  vertex: "geekblue",
  anthropic: "purple",
  bedrock: "orange",
  azureOpenAI: "blue",
};

// ---------------------------------------------------------------------------
// Extract available models from config
// ---------------------------------------------------------------------------

function extractModels(config: any): PlaygroundModel[] {
  const models: PlaygroundModel[] = [];
  const seen = new Set<string>();

  // 1) Top-level llm.models[] — requests go to the gateway at llm.port
  if (config?.llm?.models) {
    const llmPort = config.llm.port ?? 3000;
    const baseUrl = `http://localhost:${llmPort}`;
    for (const m of config.llm.models) {
      const label = m.name;
      if (!label || seen.has(label)) continue;
      seen.add(label);

      models.push({
        label,
        // params.model is the actual model forwarded to the provider
        defaultModel: m.params?.model ?? "",
        provider: m.provider ?? "unknown",
        baseUrl,
      });
    }
  }

  // 2) binds → listeners → routes → backends → ai providers
  if (config?.binds) {
    for (const bind of config.binds) {
      const port = bind.port;
      for (const listener of bind.listeners ?? []) {
        for (const route of [...(listener.routes ?? []), ...(listener.tcpRoutes ?? [])]) {
          for (const backend of route.backends ?? []) {
            const ai = backend.ai;
            if (!ai) continue;
            const providers = ai.groups
              ? ai.groups.flatMap((g: any) => g.providers ?? [])
              : [ai];
            for (const p of providers) {
              const providerEntry = p.provider ?? p;
              for (const [providerName, providerConfig] of Object.entries(providerEntry)) {
                const model = (providerConfig as any)?.model;
                if (!model || seen.has(model)) continue;
                seen.add(model);
                const baseUrl = `http://localhost:${port}`;
                models.push({
                  label: model,
                  defaultModel: model,
                  provider: providerName,
                  baseUrl,
                });
              }
            }
          }
        }
      }
    }
  }

  return models;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export const LLMPlaygroundPage = () => {
  const { data: config, isLoading } = useConfig();
  const [selectedLabel, setSelectedLabel] = useState<string | null>(null);
  const [modelOverride, setModelOverride] = useState("");
  const [prompt, setPrompt] = useState("");
  const [messages, setMessages] = useState<Message[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chatEndRef = useRef<HTMLDivElement>(null);

  const models = config ? extractModels(config) : [];
  const selectedModel = models.find((m) => m.label === selectedLabel) ?? null;
  /** The model name actually sent in the request body */
  const effectiveModel = modelOverride.trim() || selectedModel?.defaultModel || selectedModel?.label || "";

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

      const res = await fetch(`${selectedModel.baseUrl}/v1/chat/completions`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: effectiveModel,
          messages: allMessages,
        }),
      });

      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `HTTP ${res.status}`);
      }

      const data = await res.json();
      const content =
        data.choices?.[0]?.message?.content ?? "(empty response)";

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
      // Remove the user message if we failed
      setMessages((prev) => prev.slice(0, -1));
      setPrompt(userMsg.content);
    } finally {
      setSending(false);
      scrollToBottom();
    }
  }, [prompt, selectedModel, effectiveModel, messages, scrollToBottom]);

  const handleClear = useCallback(() => {
    setMessages([]);
    setPrompt("");
    setError(null);
  }, []);

  if (isLoading) {
    return (
      <Container>
        <PageTitle>LLM Playground</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
        </div>
      </Container>
    );
  }

  return (
    <Container>
      <div>
        <PageTitle>LLM Playground</PageTitle>
        <PageSubtitle>
          Send chat completions requests to your configured LLM models
        </PageSubtitle>
      </div>

      <PlaygroundLayout>
        {/* Settings Panel */}
        <Card title="Settings" size="small">
          <SidebarSection>
            <div>
              <SectionLabel>Configuration</SectionLabel>
              {models.length === 0 ? (
                <Text type="secondary" style={{ fontSize: 13 }}>
                  No models configured. Add an LLM configuration to get started.
                </Text>
              ) : (
                <Select
                  style={{ width: "100%" }}
                  placeholder="Select a configuration"
                  value={selectedLabel}
                  onChange={(label) => {
                    setSelectedLabel(label);
                    const m = models.find((m) => m.label === label);
                    setModelOverride(m?.defaultModel ?? "");
                  }}
                  options={models.map((m) => ({
                    label: (
                      <span
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 8,
                        }}
                      >
                        <Tag
                          color={PROVIDER_COLORS[m.provider] ?? "default"}
                          style={{ fontSize: 10, marginRight: 0 }}
                        >
                          {m.provider}
                        </Tag>
                        {m.label}
                      </span>
                    ),
                    value: m.label,
                  }))}
                />
              )}
            </div>

            {selectedModel && (
              <>
                <div>
                  <SectionLabel>Model Name</SectionLabel>
                  <Input
                    placeholder="e.g., smallthinker, gpt-4"
                    value={modelOverride}
                    onChange={(e) => setModelOverride(e.target.value)}
                  />
                  <Text
                    type="secondary"
                    style={{ fontSize: 11, marginTop: 4, display: "block" }}
                  >
                    The model name sent in the request body
                  </Text>
                </div>

                <div>
                  <SectionLabel>Endpoint</SectionLabel>
                  <EndpointInfo>
                    {selectedModel.baseUrl}/v1/chat/completions
                  </EndpointInfo>
                </div>
              </>
            )}

            <Button
              size="small"
              icon={<Trash2 size={14} />}
              onClick={handleClear}
              disabled={messages.length === 0 && !prompt}
            >
              Clear Chat
            </Button>
          </SidebarSection>
        </Card>

        {/* Chat Panel */}
        <Card
          title={
            <span
              style={{ display: "flex", alignItems: "center", gap: 8 }}
            >
              <Brain size={16} />
              Chat
              {selectedModel && effectiveModel && (
                <Tag
                  color={PROVIDER_COLORS[selectedModel.provider] ?? "default"}
                  style={{ fontSize: 11, marginLeft: 4 }}
                >
                  {effectiveModel}
                </Tag>
              )}
            </span>
          }
          styles={{
            body: {
              display: "flex",
              flexDirection: "column",
              minHeight: 500,
            },
          }}
        >
          <ChatContainer>
            {messages.length === 0 && !sending && (
              <EmptyState>
                {models.length === 0
                  ? "Configure LLM models to start chatting"
                  : selectedModel
                    ? "Type a message to start"
                    : "Select a model to begin"}
              </EmptyState>
            )}
            {messages.map((msg, idx) => (
              <MessageBubble key={idx} role={msg.role}>
                {msg.role === "assistant" ? (
                  <MarkdownContent>
                    <ReactMarkdown>{msg.content}</ReactMarkdown>
                  </MarkdownContent>
                ) : (
                  msg.content
                )}
              </MessageBubble>
            ))}
            {sending && (
              <MessageBubble role="assistant">
                <Spin size="small" />
              </MessageBubble>
            )}
            <div ref={chatEndRef} />
          </ChatContainer>

          {error && (
            <Text
              type="danger"
              style={{ fontSize: 13, marginBottom: 8 }}
            >
              {error}
            </Text>
          )}

          <InputRow>
            <TextArea
              placeholder={
                !selectedModel
                  ? "Select a model first…"
                  : "Type your message… (Enter to send, Shift+Enter for newline)"
              }
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  handleSend();
                }
              }}
              autoSize={{ minRows: 1, maxRows: 4 }}
              disabled={!selectedModel || sending}
            />
            <Button
              type="primary"
              icon={<Send size={16} />}
              onClick={handleSend}
              loading={sending}
              disabled={!selectedModel || !effectiveModel || !prompt.trim()}
              style={{ height: "auto", alignSelf: "flex-end" }}
            />
          </InputRow>
        </Card>
      </PlaygroundLayout>
    </Container>
  );
};
