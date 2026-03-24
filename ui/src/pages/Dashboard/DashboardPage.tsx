import styled from "@emotion/styled";
import { Card, Col, Row, Spin, Statistic, Tag, Tooltip } from "antd";
import {
  Brain,
  Headphones,
  Network,
  Route,
  Server,
  Shield,
  Workflow,
} from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useConfig, useLLMConfig, useMCPConfig } from "../../api";
import { StyledAlert } from "../../components/StyledAlert";
import { useTrafficHierarchy } from "../Traffic/hooks/useTrafficHierarchy";

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

const SectionCard = styled(Card)`
  cursor: pointer;
  transition: all 0.15s ease;

  &:hover {
    border-color: var(--color-primary);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
    transform: translateY(-1px);
  }

  .ant-card-body {
    padding: var(--spacing-lg);
  }
`;

const StatCard = styled(Card)`
  .ant-card-body {
    padding: var(--spacing-lg);
  }
  height: 100%;
`;

const IconBox = styled.div<{ color?: string }>`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 44px;
  height: 44px;
  border-radius: 10px;
  background: var(--color-bg-hover);
  color: ${({ color }) => color ?? "var(--color-primary)"};
  flex-shrink: 0;
`;

const IconLabel = styled.div`
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
  font-size: var(--font-size-sm);
  color: var(--color-text-secondary);
`;

export const DashboardPage = () => {
  const navigate = useNavigate();
  const { error: configError, isLoading: configLoading } = useConfig();
  const hierarchy = useTrafficHierarchy();
  const { data: llm } = useLLMConfig();
  const { data: mcp } = useMCPConfig();

  const isLoading = configLoading || hierarchy.isLoading;

  if (configError) {
    return (
      <Container>
        <PageTitle>Home</PageTitle>
        <StyledAlert
          message="Error Loading Configuration"
          description={configError.message || "Failed to load configuration"}
          type="error"
          showIcon
        />
      </Container>
    );
  }

  if (isLoading) {
    return (
      <Container>
        <PageTitle>Home</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
        </div>
      </Container>
    );
  }

  const { stats } = hierarchy;
  const llmModelCount = llm?.models?.length ?? 0;
  const mcpTargetCount = mcp?.targets?.length ?? 0;

  const sections = [
    {
      icon: <Workflow size={22} />,
      title: "Traffic",
      description: "Manage port binds, listeners, and routing rules",
      path: "/traffic",
      stats: [
        { label: "Binds", value: stats.totalBinds },
        { label: "Listeners", value: stats.totalListeners },
        { label: "Routes", value: stats.totalRoutes },
      ],
      status:
        stats.totalValidationErrors > 0
          ? {
              color: "warning" as const,
              text: `${stats.totalValidationErrors} issue${stats.totalValidationErrors !== 1 ? "s" : ""}`,
            }
          : stats.totalListeners > 0
            ? { color: "success" as const, text: "Healthy" }
            : null,
    },
    {
      icon: <Brain size={22} />,
      title: "LLM",
      description: "Configure large language model providers and models",
      path: "/llm",
      stats: [
        { label: "Models", value: llmModelCount },
        { label: "Policies", value: llm?.policies ? 1 : 0 },
      ],
      status:
        llmModelCount > 0
          ? { color: "success" as const, text: "Configured" }
          : null,
    },
    {
      icon: <Network size={22} />,
      title: "MCP",
      description: "Model Context Protocol server targets and configuration",
      path: "/mcp",
      stats: [
        { label: "Targets", value: mcpTargetCount },
        ...(mcp?.port ? [{ label: "Port", value: mcp.port }] : []),
      ],
      status:
        mcpTargetCount > 0
          ? { color: "success" as const, text: "Configured" }
          : null,
    },
  ];

  const quickStats = [
    {
      icon: <Network size={16} />,
      label: "Port Binds",
      value: stats.totalBinds,
      path: "/traffic",
    },
    {
      icon: <Headphones size={16} />,
      label: "Listeners",
      value: stats.totalListeners,
      path: "/traffic",
    },
    {
      icon: <Route size={16} />,
      label: "Routes",
      value: stats.totalRoutes,
      path: "/traffic",
    },
    {
      icon: <Server size={16} />,
      label: "Named Backends",
      value: stats.totalBackends,
      path: "/traffic",
    },
    {
      icon: <Brain size={16} />,
      label: "LLM Models",
      value: llmModelCount,
      path: "/llm/models",
    },
    {
      icon: <Network size={16} />,
      label: "MCP Targets",
      value: mcpTargetCount,
      path: "/mcp/servers",
    },
    {
      icon: <Shield size={16} />,
      label: "Issues",
      value: stats.totalValidationErrors,
      path: "/traffic",
      warn: stats.totalValidationErrors > 0,
    },
  ];

  return (
    <Container>
      <div>
        <PageTitle>Home</PageTitle>
        <PageSubtitle>AgentGateway configuration overview</PageSubtitle>
      </div>

      {/* Quick stats bar */}
      <Row gutter={[12, 12]}>
        {quickStats.map((s) => (
          <Col xs={12} sm={8} lg={3} key={s.label}>
            <Tooltip title={`Go to ${s.label}`}>
              <StatCard
                hoverable
                style={{ cursor: "pointer" }}
                onClick={() => navigate(s.path)}
              >
                <IconLabel>
                  {s.icon}
                  {s.label}
                </IconLabel>
                <Statistic
                  value={s.value}
                  valueStyle={{
                    fontSize: 24,
                    color: s.warn
                      ? "var(--color-warning)"
                      : "var(--color-primary)",
                  }}
                />
              </StatCard>
            </Tooltip>
          </Col>
        ))}
      </Row>

      {/* Sections */}
      <Row gutter={[16, 16]}>
        {sections.map((section) => (
          <Col xs={24} md={8} key={section.title}>
            <SectionCard onClick={() => navigate(section.path)}>
              <div
                style={{ display: "flex", flexDirection: "column", gap: 16 }}
              >
                {/* Header */}
                <div
                  style={{ display: "flex", alignItems: "flex-start", gap: 12 }}
                >
                  <IconBox>{section.icon}</IconBox>
                  <div style={{ flex: 1 }}>
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: 8,
                        marginBottom: 2,
                      }}
                    >
                      <span style={{ fontWeight: 600, fontSize: 16 }}>
                        {section.title}
                      </span>
                      {section.status && (
                        <Tag
                          color={section.status.color}
                          bordered={false}
                          style={{ fontSize: 11 }}
                        >
                          {section.status.text}
                        </Tag>
                      )}
                    </div>
                    <div
                      style={{
                        fontSize: 13,
                        color: "var(--color-text-secondary)",
                      }}
                    >
                      {section.description}
                    </div>
                  </div>
                </div>

                {/* Mini stats */}
                <div
                  style={{
                    display: "flex",
                    gap: 16,
                    paddingTop: 12,
                    borderTop: "1px solid var(--color-border-secondary)",
                  }}
                >
                  {section.stats.map((s) => (
                    <div key={s.label}>
                      <div
                        style={{
                          fontSize: 20,
                          fontWeight: 600,
                          color: "var(--color-primary)",
                        }}
                      >
                        {s.value}
                      </div>
                      <div
                        style={{
                          fontSize: 12,
                          color: "var(--color-text-secondary)",
                        }}
                      >
                        {s.label}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </SectionCard>
          </Col>
        ))}
      </Row>
    </Container>
  );
};
