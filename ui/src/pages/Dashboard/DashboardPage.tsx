import styled from "@emotion/styled";
import { Button, Card, Col, Row, Spin, Statistic, Tag, Tooltip } from "antd";
import {
  Brain,
  Headphones,
  Network,
  Route,
  Server,
  Shield,
  Workflow
} from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useConfig, useLLMConfig, useMCPConfig, useXdsMode } from "../../api";
import { AgentgatewayLogo } from "../../components/AgentgatewayLogo";
import { StyledAlert } from "../../components/StyledAlert";
import { useTrafficHierarchy } from "../../components/TrafficHierarchy";

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

const CTAHeader = styled.div`
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 24px;
  border-radius: 12px;
  background: linear-gradient(135deg, var(--color-bg-hover) 0%, var(--color-bg-container) 100%);
  border: 1px solid var(--color-border-secondary);
  text-align: center;
  gap: 12px;
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
  const { xdsMode } = useXdsMode();


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
      path: "/traffic-configuration",
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
      path: "/llm-configuration",
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
      path: "/mcp-configuration",
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
      path: "/traffic-configuration",
    },
    {
      icon: <Headphones size={16} />,
      label: "Listeners",
      value: stats.totalListeners,
      path: "/traffic-configuration",
    },
    {
      icon: <Route size={16} />,
      label: "Routes",
      value: stats.totalRoutes,
      path: "/traffic-configuration",
    },
    {
      icon: <Server size={16} />,
      label: "Named Backends",
      value: stats.totalBackends,
      path: "/traffic-configuration",
    },
    {
      icon: <Brain size={16} />,
      label: "LLM Models",
      value: llmModelCount,
      path: "/llm-configuration",
    },
    {
      icon: <Network size={16} />,
      label: "MCP Targets",
      value: mcpTargetCount,
      path: "/mcp-configuration",
    },
    {
      icon: <Shield size={16} />,
      label: "Issues",
      value: stats.totalValidationErrors,
      path: "/traffic-configuration",
      warn: stats.totalValidationErrors > 0,
    },
  ];

  const ctaDescription = "Connect an LLM model and MCP targets to get started with agentgateway."
  const ctaDescriptionXdsMode = "Configuration is managed by a remote control plane. Edits are disabled.";

  return (
    <Container>
      
      {/* Call to action header */}
      <CTAHeader data-testid="xds-call-to-action-card">
        <div style={{ width: 40, height: 40 }}>
          <AgentgatewayLogo />
        </div>
          <div>
          <div style={{ fontSize: 20, fontWeight: 600, marginBottom: 6 }}>
            {xdsMode ? "agentgateway: xDS mode is enabled" : "Get started with agentgateway"}
          </div>
          <div style={{ fontSize: 13, color: "var(--color-text-secondary)", maxWidth: 420 }}>
            {xdsMode ? ctaDescriptionXdsMode : ctaDescription}
          </div>
        </div>
        {!xdsMode && (
          <div style={{ display: 'flex', gap: 8}}>  
            <Button type="primary" size="large" onClick={() => navigate("/llm-setup-wizard")}>
              Open LLM Setup Wizard →
            </Button>
            <Button type="primary" size="large" onClick={() => navigate("/mcp-setup-wizard")}>
              Open MCP Setup Wizard →
            </Button>
          </div>
        )}
      </CTAHeader>

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
                data-testid={`${section.title.toLowerCase()}`}
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
