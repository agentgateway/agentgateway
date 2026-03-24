import styled from "@emotion/styled";
import { Button, Card, Input, Select, Spin, Tag } from "antd";
import { Brain, Send } from "lucide-react";
import { useState } from "react";
import { useLLMConfig } from "../../api";

const { TextArea } = Input;

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

const ChatContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-sm);
  min-height: 400px;
`;

const MessageBubble = styled.div<{ role: "user" | "assistant" }>`
  padding: 12px 16px;
  border-radius: 12px;
  max-width: 85%;
  font-size: 14px;
  line-height: 1.5;
  align-self: ${({ role }) => (role === "user" ? "flex-end" : "flex-start")};
  background: ${({ role }) =>
    role === "user"
      ? "var(--color-primary)"
      : "var(--color-bg-hover)"};
  color: ${({ role }) =>
    role === "user" ? "#fff" : "var(--color-text-base)"};
`;

const PROVIDER_COLORS: Record<string, string> = {
  openAI: "blue",
  gemini: "cyan",
  vertex: "geekblue",
  anthropic: "purple",
  bedrock: "orange",
  azureOpenAI: "blue",
};

interface Message {
  role: "user" | "assistant";
  content: string;
}

export const LLMPlaygroundPage = () => {
  const { data: llm, isLoading } = useLLMConfig();
  const [selectedModel, setSelectedModel] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [messages, setMessages] = useState<Message[]>([]);
  const [sending, setSending] = useState(false);

  const models = llm?.models ?? [];

  const handleSend = async () => {
    if (!prompt.trim() || !selectedModel) return;
    const userMsg: Message = { role: "user", content: prompt.trim() };
    setMessages((prev) => [...prev, userMsg]);
    setPrompt("");
    setSending(true);

    // Simulate a response (real implementation would call the LLM API)
    await new Promise((r) => setTimeout(r, 800));
    setMessages((prev) => [
      ...prev,
      {
        role: "assistant",
        content:
          "LLM Playground requires a connected LLM backend. This is a preview of the interface — configure your models and backend connection to enable real interactions.",
      },
    ]);
    setSending(false);
  };

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
        <PageSubtitle>Test LLM model interactions</PageSubtitle>
      </div>

      <PlaygroundLayout>
        {/* Settings Panel */}
        <Card title="Settings" size="small">
          <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
            <div>
              <div
                style={{
                  fontSize: 12,
                  fontWeight: 600,
                  color: "var(--color-text-secondary)",
                  marginBottom: 6,
                  textTransform: "uppercase",
                  letterSpacing: "0.05em",
                }}
              >
                Model
              </div>
              {models.length === 0 ? (
                <div
                  style={{
                    fontSize: 13,
                    color: "var(--color-text-secondary)",
                    padding: "8px 0",
                  }}
                >
                  No models configured.{" "}
                  <a href="/llm/models">Add models</a> to get started.
                </div>
              ) : (
                <Select
                  style={{ width: "100%" }}
                  placeholder="Select a model"
                  value={selectedModel}
                  onChange={setSelectedModel}
                  options={models.map((m) => ({
                    label: (
                      <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
                        <Tag
                          color={PROVIDER_COLORS[m.provider] ?? "default"}
                          style={{ fontSize: 10, marginRight: 0 }}
                        >
                          {m.provider}
                        </Tag>
                        {m.name}
                      </span>
                    ),
                    value: m.name,
                  }))}
                />
              )}
            </div>

            {selectedModel && (
              <div>
                <div
                  style={{
                    fontSize: 12,
                    fontWeight: 600,
                    color: "var(--color-text-secondary)",
                    marginBottom: 6,
                    textTransform: "uppercase",
                    letterSpacing: "0.05em",
                  }}
                >
                  Selected
                </div>
                {(() => {
                  const m = models.find((m) => m.name === selectedModel);
                  if (!m) return null;
                  return (
                    <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                      <Tag color={PROVIDER_COLORS[m.provider] ?? "default"}>
                        {m.provider}
                      </Tag>
                      {m.params?.model && (
                        <div style={{ fontSize: 12, color: "var(--color-text-secondary)" }}>
                          Forwards as: <code>{m.params.model}</code>
                        </div>
                      )}
                    </div>
                  );
                })()}
              </div>
            )}

            <Button
              size="small"
              onClick={() => {
                setMessages([]);
                setPrompt("");
              }}
              disabled={messages.length === 0}
            >
              Clear Chat
            </Button>
          </div>
        </Card>

        {/* Chat Panel */}
        <Card
          title={
            <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <Brain size={16} />
              Chat
            </span>
          }
          styles={{ body: { display: "flex", flexDirection: "column", gap: 12, minHeight: 400 } }}
        >
          <ChatContainer>
            {messages.length === 0 && !sending && (
              <div
                style={{
                  flex: 1,
                  display: "flex",
                  alignItems: "center",
                  justifyContent: "center",
                  color: "var(--color-text-tertiary)",
                  fontSize: 14,
                }}
              >
                {models.length === 0
                  ? "Configure LLM models to start chatting"
                  : selectedModel
                    ? "Type a message to start"
                    : "Select a model to begin"}
              </div>
            )}
            {messages.map((msg, idx) => (
              <MessageBubble key={idx} role={msg.role}>
                {msg.content}
              </MessageBubble>
            ))}
            {sending && (
              <MessageBubble role="assistant">
                <Spin size="small" />
              </MessageBubble>
            )}
          </ChatContainer>

          <div style={{ display: "flex", gap: 8, marginTop: "auto" }}>
            <TextArea
              placeholder={
                !selectedModel
                  ? "Select a model first…"
                  : "Type your message…"
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
              disabled={!selectedModel || !prompt.trim()}
            />
          </div>
        </Card>
      </PlaygroundLayout>
    </Container>
  );
};
