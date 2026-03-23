import styled from "@emotion/styled";
import { Card, Col, Descriptions, Empty, Row, Spin, Tag, Tooltip } from "antd";
import { Brain, Key, Shield } from "lucide-react";
import { useState } from "react";
import { useLLMConfig } from "../../api";
import type { LocalLLMModels } from "../../api/types";

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

const ModelCard = styled(Card)<{ selected?: boolean }>`
  cursor: pointer;
  transition: all 0.15s ease;
  border-color: ${({ selected }) =>
    selected ? "var(--color-primary)" : "var(--color-border-base)"};
  box-shadow: ${({ selected }) =>
    selected ? "0 0 0 2px color-mix(in srgb, var(--color-primary) 20%, transparent)" : "none"};

  &:hover {
    border-color: var(--color-primary);
  }
`;

const PROVIDER_COLORS: Record<string, string> = {
  openAI: "blue",
  gemini: "cyan",
  vertex: "geekblue",
  anthropic: "purple",
  bedrock: "orange",
  azureOpenAI: "blue",
};

function ModelDetailPanel({ model }: { model: LocalLLMModels }) {
  const params = model.params;
  const hasGuardrails = !!model.guardrails;
  const hasMatches = (model.matches?.length ?? 0) > 0;

  const items = [
    { key: "name", label: "Model Name (matched)", children: <code>{model.name}</code> },
    {
      key: "provider",
      label: "Provider",
      children: (
        <Tag color={PROVIDER_COLORS[model.provider] ?? "default"}>{model.provider}</Tag>
      ),
    },
    ...(params?.model
      ? [{ key: "model", label: "Forwarded Model", children: <code>{params.model}</code> }]
      : []),
    ...(params?.apiKey
      ? [
          {
            key: "apiKey",
            label: "API Key",
            children: (
              <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
                <Key size={14} />
                <span style={{ fontFamily: "monospace" }}>
                  {"•".repeat(12)}
                </span>
              </span>
            ),
          },
        ]
      : []),
    ...(params?.azureHost
      ? [{ key: "azureHost", label: "Azure Host", children: params.azureHost }]
      : []),
    ...(params?.awsRegion
      ? [{ key: "awsRegion", label: "AWS Region", children: params.awsRegion }]
      : []),
    ...(params?.vertexProject
      ? [{ key: "vertexProject", label: "Vertex Project", children: params.vertexProject }]
      : []),
    {
      key: "guardrails",
      label: "Guardrails",
      children: hasGuardrails ? (
        <Tag color="warning" icon={<Shield size={12} style={{ marginRight: 4 }} />}>
          Configured
        </Tag>
      ) : (
        <span style={{ color: "var(--color-text-tertiary)" }}>None</span>
      ),
    },
    {
      key: "matches",
      label: "Extra Match Conditions",
      children: hasMatches ? (
        <Tag>{model.matches!.length} condition{model.matches!.length !== 1 ? "s" : ""}</Tag>
      ) : (
        <span style={{ color: "var(--color-text-tertiary)" }}>Model name only</span>
      ),
    },
  ];

  return (
    <Card title={`Model: ${model.name}`} variant="borderless">
      <Descriptions bordered column={1} size="small" items={items} />
    </Card>
  );
}

export const LLMModelsPage = () => {
  const { data: llm, isLoading } = useLLMConfig();
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);

  if (isLoading) {
    return (
      <Container>
        <PageTitle>LLM Models</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
        </div>
      </Container>
    );
  }

  const models = llm?.models ?? [];
  const selectedModel = selectedIdx !== null ? models[selectedIdx] : null;

  return (
    <Container>
      <div>
        <PageTitle>LLM Models</PageTitle>
        <PageSubtitle>
          {models.length > 0
            ? `${models.length} model${models.length !== 1 ? "s" : ""} configured`
            : "No models configured yet"}
        </PageSubtitle>
      </div>

      {models.length === 0 ? (
        <Card>
          <Empty
            description={
              <span>
                No LLM models configured.{" "}
                <span style={{ color: "var(--color-text-secondary)" }}>
                  Add models to your config file under the <code>llm.models</code> key.
                </span>
              </span>
            }
            image={Empty.PRESENTED_IMAGE_SIMPLE}
          />
        </Card>
      ) : (
        <Row gutter={[16, 16]}>
          <Col xs={24} lg={10}>
            <Card title="Models">
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {models.map((model, idx) => (
                  <ModelCard
                    key={idx}
                    selected={selectedIdx === idx}
                    size="small"
                    onClick={() => setSelectedIdx(idx === selectedIdx ? null : idx)}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 10,
                      }}
                    >
                      <div
                        style={{
                          width: 32,
                          height: 32,
                          borderRadius: 6,
                          background: "var(--color-bg-hover)",
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "center",
                          color: "var(--color-primary)",
                          flexShrink: 0,
                        }}
                      >
                        <Brain size={16} />
                      </div>
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div
                          style={{
                            fontWeight: 500,
                            overflow: "hidden",
                            textOverflow: "ellipsis",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {model.name}
                        </div>
                        {model.params?.model && (
                          <div
                            style={{
                              fontSize: 11,
                              color: "var(--color-text-secondary)",
                            }}
                          >
                            → {model.params.model}
                          </div>
                        )}
                      </div>
                      <Tooltip title={model.provider}>
                        <Tag
                          color={PROVIDER_COLORS[model.provider] ?? "default"}
                          style={{ flexShrink: 0 }}
                        >
                          {model.provider}
                        </Tag>
                      </Tooltip>
                      {model.guardrails && (
                        <Tooltip title="Has guardrails">
                          <Shield
                            size={14}
                            style={{ color: "var(--color-warning)", flexShrink: 0 }}
                          />
                        </Tooltip>
                      )}
                    </div>
                  </ModelCard>
                ))}
              </div>
            </Card>
          </Col>

          <Col xs={24} lg={14}>
            {selectedModel ? (
              <ModelDetailPanel model={selectedModel} />
            ) : (
              <Card variant="borderless">
                <Empty
                  description="Select a model to view details"
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                />
              </Card>
            )}
          </Col>
        </Row>
      )}

      {/* LLM-level policies */}
      {llm?.policies && (
        <Card title="LLM-Level Policies">
          <div
            style={{
              display: "flex",
              flexWrap: "wrap",
              gap: 8,
              alignItems: "center",
            }}
          >
            {llm.policies.jwtAuth && <Tag color="blue">JWT Auth</Tag>}
            {llm.policies.extAuthz && <Tag color="purple">Ext Authz</Tag>}
            {llm.policies.extProc && <Tag color="cyan">Ext Processor</Tag>}
            {llm.policies.transformations && <Tag color="geekblue">Transformations</Tag>}
            {llm.policies.basicAuth && <Tag color="orange">Basic Auth</Tag>}
            {llm.policies.apiKey && <Tag color="gold">API Key Auth</Tag>}
            {llm.policies.authorization && <Tag color="red">Authorization</Tag>}
          </div>
        </Card>
      )}
    </Container>
  );
};
