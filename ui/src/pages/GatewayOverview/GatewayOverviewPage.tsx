import styled from "@emotion/styled";
import { Button, Card, Col, Row, Spin, Tag } from "antd";
import {
  Bot,
  Network,
  Server
} from "lucide-react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useConfig, useLLMConfig, useMCPConfig, useXdsMode } from "../../api";
import { updateConfig } from "../../api/config";
import { AgentgatewayLogo } from "../../components/AgentgatewayLogo";
import { StyledAlert } from "../../components/StyledAlert";
import { useTrafficHierarchy } from "../../components/TrafficHierarchy";

// region Styles

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

const WelcomeCard = styled(Card)`
  text-align: center;

  .ant-card-body {
    padding: 48px 24px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
  }
`;

const OptionCard = styled(Card)<{ selected?: boolean }>`
  cursor: pointer;
  transition: all 0.15s ease;
  border: 2px solid ${({ selected }) => selected ? "var(--color-primary)" : "var(--color-border-secondary)"};
  background: ${({ selected }) => selected ? "color-mix(in srgb, var(--color-primary) 8%, var(--color-bg-container))" : "var(--color-bg-container)"};

  &:hover {
    border-color: var(--color-primary);
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.08);
  }

  .ant-card-body {
    padding: var(--spacing-lg);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    text-align: center;
  }
`;

const SurfaceRow = styled.div`
  display: flex;
  flex-direction: column;
  gap: 0;
  border: 1px solid var(--color-border-base);
  border-radius: var(--border-radius-md);
  overflow: hidden;
`;

const SurfaceRowHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 20px;
  background: var(--color-bg-container);
  border-bottom: 1px solid var(--color-border-secondary);

  &:last-child {
    border-bottom: none;
  }
`;

const SurfaceRowLeft = styled.div`
  display: flex;
  align-items: center;
  gap: 12px;
`;

const SurfaceRowRight = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
`;

const IconBox = styled.div<{ color?: string }>`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: 8px;
  color: ${({ color }) => color ?? "var(--primary)"};
  flex-shrink: 0;
`;

const StatPill = styled.span`
  font-size: 12px;
  color: var(--color-text-secondary);
  background: var(--color-bg-hover);
  border-radius: 20px;
  padding: 2px 10px;
`;

// endregion Styles

// region Helpers

type SurfaceKey = "llm" | "mcp" | "traffic";

// endregion Helpers

// region Component

export const GatewayOverviewPage = () => {
  const navigate = useNavigate();
  const { data: config, error: configError, isLoading: configLoading, mutate } = useConfig();
  const hierarchy = useTrafficHierarchy();
  const { data: llm } = useLLMConfig();
  const { data: mcp } = useMCPConfig();
  const { xdsMode } = useXdsMode();

  const [selectedSurfaces, setSelectedSurfaces] = useState<Set<SurfaceKey>>(new Set());
  const [isEnabling, setIsEnabling] = useState(false);

  const isLoading = configLoading || hierarchy.isLoading;

  if (configError) {
    return (
      <Container>
        <PageTitle>Gateway Overview</PageTitle>
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
        <PageTitle>Gateway Overview</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
        </div>
      </Container>
    );
  }

  const hasLLM = !!config?.llm;
  const hasMCP = !!config?.mcp;
  const hasTraffic = Array.isArray(config?.binds);
  const hasAnySurface = hasLLM || hasMCP || hasTraffic;

  // Onboarding / startup flow
  if (!hasAnySurface && !xdsMode) {
    const toggleSurface = (key: SurfaceKey) => {
      setSelectedSurfaces((prev) => {
        const next = new Set(prev);
        if (next.has(key)) next.delete(key);
        else next.add(key);
        return next;
      });
    };

    const handleContinue = async () => {
      setIsEnabling(true);
      try {
        const newConfig: any = { ...(config ?? {}) };
        if (selectedSurfaces.has("llm")) newConfig.llm = { port: 4000, models: [] };
        if (selectedSurfaces.has("mcp")) newConfig.mcp = { port: 3000, targets: [] };
        if (selectedSurfaces.has("traffic")) newConfig.binds = [];
        await updateConfig(newConfig);
        await mutate();
      } finally {
        setIsEnabling(false);
      }
    };

    const surfaces: { key: SurfaceKey; icon: React.ReactNode; label: string; description: string }[] = [
      { key: "llm", icon: <Bot size={28} />, label: "LLM", description: "Route and observe LLM traffic" },
      { key: "mcp", icon: <Server size={28} />, label: "MCP", description: "Expose MCP tool servers" },
      { key: "traffic", icon: <Network size={28} />, label: "APIs", description: "Proxy traditional API traffic" },
    ];

    return (
      <Container>
        <WelcomeCard>
          <div style={{ width: 48, height: 48 }}>
            <AgentgatewayLogo />
          </div>
          <div>
            <div style={{ fontSize: 22, fontWeight: 700, marginBottom: 8 }}>
              Welcome to Agentgateway
            </div>
            <div style={{ fontSize: 14, color: "var(--color-text-secondary)", maxWidth: 520, margin: "0 auto" }}>
              Agentgateway is a gateway that can route, secure, and observe LLM, MCP, and traditional API traffic.
              To get started, enable the things you want to operate.
            </div>
          </div>

          <Row gutter={[16, 16]} style={{ marginTop: 8, width: "100%", maxWidth: 560 }}>
            {surfaces.map((s) => (
              <Col xs={24} sm={8} key={s.key}>
                <OptionCard
                  selected={selectedSurfaces.has(s.key)}
                  onClick={() => toggleSurface(s.key)}
                >
                  <IconBox>{s.icon}</IconBox>
                  <div style={{ fontWeight: 600, fontSize: 16 }}>{s.label}</div>
                  <div style={{ fontSize: 12, color: "var(--color-text-secondary)" }}>{s.description}</div>
                </OptionCard>
              </Col>
            ))}
          </Row>

          {selectedSurfaces.size > 0 && (
            <div style={{ display: "flex", flexDirection: "column", alignItems: "center", gap: 8 }}>
              <div style={{ fontSize: 13, color: "var(--color-text-secondary)" }}>
                {selectedSurfaces.size} of 3 enabled
              </div>
              <Button
                type="primary"
                size="large"
                loading={isEnabling}
                onClick={handleContinue}
              >
                Continue
              </Button>
            </div>
          )}
        </WelcomeCard>
      </Container>
    );
  }

  // Main dashboard
  const { stats } = hierarchy;
  const llmModelCount = llm?.models?.length ?? 0;
  const mcpTargetCount = mcp?.targets?.length ?? 0;

  // Compute warnings
  const warnings: string[] = [];
  if (stats.totalValidationErrors > 0) {
    warnings.push(`${stats.totalValidationErrors} traffic validation issue${stats.totalValidationErrors !== 1 ? "s" : ""}`);
  }
  const modelNames = (llm?.models ?? []).map((m: any) => m.name).filter(Boolean);
  const duplicateModelNames = modelNames.filter((n: string, i: number) => modelNames.indexOf(n) !== i);
  if (duplicateModelNames.length > 0) {
    warnings.push(`Duplicate LLM model name${duplicateModelNames.length !== 1 ? "s" : ""}: ${[...new Set(duplicateModelNames)].join(", ")}`);
  }

  return (
    <Container>
      <PageTitle>Gateway Overview</PageTitle>

      {warnings.length > 0 && (
        <StyledAlert
          message={`${warnings.length} warning${warnings.length !== 1 ? "s" : ""}`}
          description={warnings.join(" · ")}
          type="warning"
          showIcon
        />
      )}

      {xdsMode && (
        <StyledAlert
          message="xDS mode is enabled"
          description="Configuration is managed by a remote control plane. Edits are disabled."
          type="info"
          showIcon
        />
      )}

      {/* LLM Row */}
      <SurfaceRow>
        <SurfaceRowHeader>
          <SurfaceRowLeft>
            <IconBox><Bot size={18} /></IconBox>
            <div>
              <div style={{ fontWeight: 600 }}>LLM</div>
              <div style={{ fontSize: 12, color: "var(--color-text-secondary)" }}>
                Large language model routing
              </div>
            </div>
          </SurfaceRowLeft>
          <SurfaceRowRight>
            {!hasLLM ? (
              <>
                <Tag bordered={false}>Not enabled</Tag>
                {!xdsMode && (
                  <Button size="small" type="primary" onClick={() => navigate("/llm-models")}>
                    Enable
                  </Button>
                )}
              </>
            ) : llmModelCount === 0 ? (
              <>
                <Tag color="warning" bordered={false}>No models</Tag>
                <Button size="small" onClick={() => navigate("/llm-models")}>
                  Add a model
                </Button>
              </>
            ) : (
              <>
                <StatPill>{llmModelCount} model{llmModelCount !== 1 ? "s" : ""}</StatPill>
                <Button size="small" onClick={() => navigate(`/llm-playground`)}>
                  Playground
                </Button>
                <Button size="small" onClick={() => navigate("/llm-monitoring")}>
                  Monitoring
                </Button>
              </>
            )}
          </SurfaceRowRight>
        </SurfaceRowHeader>
      </SurfaceRow>

      {/* MCP Row */}
      <SurfaceRow>
        <SurfaceRowHeader>
          <SurfaceRowLeft>
            <IconBox><Server size={18} /></IconBox>
            <div>
              <div style={{ fontWeight: 600 }}>MCP</div>
              <div style={{ fontSize: 12, color: "var(--color-text-secondary)" }}>
                Model Context Protocol servers
              </div>
            </div>
          </SurfaceRowLeft>
          <SurfaceRowRight>
            {!hasMCP ? (
              <>
                <Tag bordered={false}>Not enabled</Tag>
                {!xdsMode && (
                  <Button size="small" type="primary" onClick={() => navigate("/mcp-servers")}>
                    Enable
                  </Button>
                )}
              </>
            ) : mcpTargetCount === 0 ? (
              <>
                <Tag color="warning" bordered={false}>No servers</Tag>
                <Button size="small" onClick={() => navigate("/mcp-servers")}>
                  Add a server
                </Button>
              </>
            ) : (
              <>
                <StatPill>{mcpTargetCount} server{mcpTargetCount !== 1 ? "s" : ""}</StatPill>
                {mcp?.port && <StatPill>port {mcp.port}</StatPill>}
                <Button size="small" onClick={() => navigate("/mcp-playground")}>
                  Playground
                </Button>
              </>
            )}
          </SurfaceRowRight>
        </SurfaceRowHeader>
      </SurfaceRow>

      {/* Traffic Row */}
      <SurfaceRow>
        <SurfaceRowHeader>
          <SurfaceRowLeft>
            <IconBox><Network size={18} /></IconBox>
            <div>
              <div style={{ fontWeight: 600 }}>Traffic</div>
              <div style={{ fontSize: 12, color: "var(--color-text-secondary)" }}>
                API routing and proxy
              </div>
            </div>
          </SurfaceRowLeft>
          <SurfaceRowRight>
            {!hasTraffic ? (
              <>
                <Tag bordered={false}>Not enabled</Tag>
                {!xdsMode && (
                  <Button size="small" type="primary" onClick={() => navigate("/traffic-listeners")}>
                    Enable
                  </Button>
                )}
              </>
            ) : stats.totalBinds === 0 ? (
              <>
                <Tag color="warning" bordered={false}>No binds</Tag>
                <Button size="small" onClick={() => navigate("/traffic-listeners")}>
                  Add a listener
                </Button>
              </>
            ) : (
              <>
                <StatPill>{stats.totalBinds} bind{stats.totalBinds !== 1 ? "s" : ""}</StatPill>
                <StatPill>{stats.totalListeners} listener{stats.totalListeners !== 1 ? "s" : ""}</StatPill>
                <StatPill>{stats.totalRoutes} route{stats.totalRoutes !== 1 ? "s" : ""}</StatPill>
                <Button size="small" onClick={() => navigate("/traffic-routes")}>
                  Routes
                </Button>
              </>
            )}
          </SurfaceRowRight>
        </SurfaceRowHeader>
      </SurfaceRow>
    </Container>
  );
};
