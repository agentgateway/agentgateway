import styled from "@emotion/styled";
import { Spin } from "antd";
import { ChatPanel } from "./ChatPanel";
import { SettingsPanel } from "./SettingsPanel";
import { usePlayground } from "./usePlayground";

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

export function LLMPlaygroundPage() {
  const {
    isLoading,
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
  } = usePlayground();

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
        <SettingsPanel
          models={models}
          selectedLabel={selectedLabel}
          selectedModel={selectedModel}
          modelOverride={modelOverride}
          messages={messages}
          prompt={prompt}
          onSelectLabel={handleSelectLabel}
          onChangeModelOverride={setModelOverride}
          onClear={handleClear}
        />

        <ChatPanel
          models={models}
          selectedModel={selectedModel}
          effectiveModel={effectiveModel}
          messages={messages}
          sending={sending}
          error={error}
          prompt={prompt}
          chatEndRef={chatEndRef}
          onPromptChange={setPrompt}
          onSend={handleSend}
        />
      </PlaygroundLayout>
    </Container>
  );
}
