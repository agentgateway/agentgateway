import { CodeOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Alert, Button, Spin } from "antd";
import { ChevronDown, ChevronRight } from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { CodeBlock } from "../../../components/CodeBlock";
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

const exampleConfig = `
  binds:
    - port: 8080
      tunnelProtocol: direct
      listeners:
        - protocol: HTTP
          name: listener
          hostname: '*'
          routes:
            - hostnames: []
              matches:
                - path:
                    pathPrefix: /
              backends:
                - ai:
                    name: ollama
                    hostOverride: localhost:11434
                    tokenize: false
                    provider:
                      openAI:
                        model: smallthinker
                  weight: 1
              name: route
              policies:
                cors:
                  allowCredentials: false
                  allowHeaders:
                    - '*'
                  allowMethods:
                    - GET
                    - POST
                    - OPTIONS
                  allowOrigins:
                    - '*'
                  exposeHeaders: []
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
    hasTopLevelLlm,
    handleSend,
    handleClear,
    handleSelectLabel,
    setModelOverride,
    setPrompt,
  } = usePlayground();
  const navigate = useNavigate();

  const [showExample, setShowExample] = useState(false);

  const showAlert = !isLoading && (hasTopLevelLlm || models.length === 0);

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
        {showAlert && (
          <Alert
            type="warning"
            showIcon
            closable
            style={{ alignItems: "flex-start" }}
            message={
              <>
                LLM Playground doesn't support root-level configuration. Configure your model with CORS at the route level using Port Bind instead.{" "}
              </>
            }
            description={
              <>
                <a 
                  href="https://agentgateway.dev/docs/standalone/latest/llm/configuration-modes/#traditional-http-routing-configuration" 
                  target="_blank"
                >
                  Learn more
                </a>
                <div style={{ marginTop: 8 }}>
                  <div style={{ display: "flex", gap: 8}}>
                    <Button
                      onClick={() => setShowExample(v => !v)}
                    >
                      {showExample ? <ChevronDown size={14} /> : <ChevronRight size={14} />} Example Config
                    </Button>
                    <Button
                      icon={<CodeOutlined />}
                      onClick={() => navigate("/traffic-configuration/editor")}
                    >
                      Editor
                    </Button>
                  </div>
                  {showExample && (
                    <div style={{ marginTop: 8 }}>
                      <CodeBlock 
                        code={exampleConfig} 
                      />
                    </div>
                  )}
                </div>
              </>
            }
          />
        )}
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
