import styled from "@emotion/styled";
import { Button, Card, Col, Empty, Row, Spin, Statistic, Tag } from "antd";
import { Brain, Boxes, Shield } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useLLMConfig } from "../../api";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
`;

const PageHeader = styled.div`
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--spacing-sm);
`;

const PageTitle = styled.h1`
  margin: 0;
  font-size: 24px;
  font-weight: 600;
`;

const PageSubtitle = styled.p`
  margin: 0;
  color: var(--color-text-secondary);
  font-size: 14px;
`;

const StatCard = styled(Card)`
  .ant-card-body {
    padding: var(--spacing-lg);
  }
  height: 100%;
`;

const IconLabel = styled.div`
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
  font-size: var(--font-size-sm);
  color: var(--color-text-secondary);
`;

const PROVIDER_COLORS: Record<string, string> = {
  openAI: "blue",
  gemini: "cyan",
  vertex: "geekblue",
  anthropic: "purple",
  bedrock: "orange",
  azureOpenAI: "blue",
};

export const LLMOverviewPage = () => {
  const navigate = useNavigate();
  const { data: llm, isLoading } = useLLMConfig();

  if (isLoading) {
    return (
      <Container>
        <PageTitle>LLM Overview</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
        </div>
      </Container>
    );
  }

  const models = llm?.models ?? [];
  const providers = [...new Set(models.map((m) => m.provider))];
  const hasPolicies = !!llm?.policies;

  return (
    <Container>
      <PageHeader>
        <div>
          <PageTitle>LLM Overview</PageTitle>
          <PageSubtitle>
            Large Language Model gateway configuration
          </PageSubtitle>
        </div>
        <Button type="primary" onClick={() => navigate("/llm/models")}>
          Manage Models
        </Button>
      </PageHeader>

      {/* Stats */}
      <Row gutter={[16, 16]}>
        <Col xs={12} sm={8}>
          <StatCard>
            <IconLabel>
              <Brain size={16} />
              Total Models
            </IconLabel>
            <Statistic
              value={models.length}
              valueStyle={{ color: "var(--color-primary)", fontSize: 28 }}
            />
          </StatCard>
        </Col>
        <Col xs={12} sm={8}>
          <StatCard>
            <IconLabel>
              <Boxes size={16} />
              Providers
            </IconLabel>
            <Statistic
              value={providers.length}
              valueStyle={{ color: "var(--color-primary)", fontSize: 28 }}
            />
          </StatCard>
        </Col>
        <Col xs={12} sm={8}>
          <StatCard>
            <IconLabel>
              <Shield size={16} />
              Policies
            </IconLabel>
            <Statistic
              value={hasPolicies ? "Active" : "None"}
              valueStyle={{
                color: hasPolicies
                  ? "var(--color-success)"
                  : "var(--color-text-tertiary)",
                fontSize: 20,
              }}
            />
          </StatCard>
        </Col>
      </Row>

      {/* Models List */}
      <Card
        title="Configured Models"
        extra={
          <Button size="small" onClick={() => navigate("/llm/models")}>
            View all
          </Button>
        }
      >
        {models.length === 0 ? (
          <Empty
            description="No LLM models configured"
            image={Empty.PRESENTED_IMAGE_SIMPLE}
          >
            <Button type="primary" onClick={() => navigate("/llm/models")}>
              Add Model
            </Button>
          </Empty>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            {models.slice(0, 5).map((model, idx) => (
              <div
                key={idx}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 12,
                  padding: "8px 0",
                  borderBottom:
                    idx < Math.min(models.length, 5) - 1
                      ? "1px solid var(--color-border-secondary)"
                      : undefined,
                }}
              >
                <div
                  style={{
                    width: 36,
                    height: 36,
                    borderRadius: 8,
                    background: "var(--color-bg-hover)",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    color: "var(--color-primary)",
                    flexShrink: 0,
                  }}
                >
                  <Brain size={18} />
                </div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontWeight: 500 }}>{model.name}</div>
                  {model.params?.model && (
                    <div
                      style={{
                        fontSize: 12,
                        color: "var(--color-text-secondary)",
                      }}
                    >
                      → {model.params.model}
                    </div>
                  )}
                </div>
                <Tag color={PROVIDER_COLORS[model.provider] ?? "default"}>
                  {model.provider}
                </Tag>
              </div>
            ))}
            {models.length > 5 && (
              <div
                style={{
                  textAlign: "center",
                  color: "var(--color-text-secondary)",
                  fontSize: 13,
                  paddingTop: 8,
                }}
              >
                +{models.length - 5} more models
              </div>
            )}
          </div>
        )}
      </Card>

      {/* Providers breakdown */}
      {providers.length > 0 && (
        <Card title="Providers">
          <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
            {providers.map((p) => {
              const count = models.filter((m) => m.provider === p).length;
              return (
                <Tag
                  key={p}
                  color={PROVIDER_COLORS[p] ?? "default"}
                  style={{ padding: "4px 12px", fontSize: 13 }}
                >
                  {p} ({count})
                </Tag>
              );
            })}
          </div>
        </Card>
      )}
    </Container>
  );
};
