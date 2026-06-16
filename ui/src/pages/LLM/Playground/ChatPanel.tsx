import styled from "@emotion/styled";
import { Button, Card, Input, Spin, Typography } from "antd";
import { Bot, Send } from "lucide-react";
import type { RefObject } from "react";
import ReactMarkdown from "react-markdown";
import { ProviderIcon } from "../../../imported/components/ProviderIcon";
import type { Message, PlaygroundModel } from "./types";

const { TextArea } = Input;
const { Text } = Typography;

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

const SendButton = styled(Button)`
  display: flex !important;
  align-items: center !important;
  justify-content: center !important;
  padding: 0 !important;

  .ant-btn-icon {
    margin: 0 !important;
    display: flex !important;
    align-items: center !important;
    justify-content: center !important;
  }
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

interface ChatPanelProps {
  models: PlaygroundModel[];
  selectedModel: PlaygroundModel | null;
  effectiveModel: string;
  messages: Message[];
  sending: boolean;
  error: string | null;
  prompt: string;
  chatEndRef: RefObject<HTMLDivElement | null>;
  onPromptChange: (value: string) => void;
  onSend: () => void;
}

export function ChatPanel({
  models,
  selectedModel,
  effectiveModel,
  messages,
  sending,
  error,
  prompt,
  chatEndRef,
  onPromptChange,
  onSend,
}: ChatPanelProps) {
  return (
    <Card
      title={
        <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Bot size={16} />
          Chat
        </span>
      }
      extra={
        selectedModel && effectiveModel && (
          <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <ProviderIcon provider={selectedModel.provider} />
            <span style={{ fontSize: 11, color: "var(--color-text-secondary)" }}>{effectiveModel}</span>
            <span style={{ fontSize: 11, color: "var(--color-text-tertiary)" }}>{selectedModel.label}</span>
          </span>
        )
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
        <Text type="danger" style={{ fontSize: 13, marginBottom: 8 }}>
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
          onChange={(e) => onPromptChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              onSend();
            }
          }}
          autoSize={{ minRows: 1, maxRows: 4 }}
          disabled={!selectedModel || sending}
        />
        <SendButton
          type="primary"
          icon={<Send size={16} />}
          onClick={onSend}
          loading={sending}
          disabled={!selectedModel || !effectiveModel || !prompt.trim()}
          style={{ height: "auto", alignSelf: "flex-end" }}
        />
      </InputRow>
    </Card>
  );
}
