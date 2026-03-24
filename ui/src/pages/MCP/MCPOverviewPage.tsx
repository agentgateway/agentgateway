import styled from "@emotion/styled";
import { Button, Card, Col, Empty, Row, Spin, Statistic, Tag } from "antd";
import { Network, Server, Workflow } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { useMCPConfig } from "../../api";
import type { LocalMcpTarget } from "../../api/types";

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

function getTargetType(target: LocalMcpTarget): string {
  const t = target as Record<string, unknown>;
  if (t["sse"]) return "SSE";
  if (t["mcp"]) return "MCP";
  if (t["stdio"]) return "STDIO";
  if (t["openapi"]) return "OpenAPI";
  return "Unknown";
}

function getTargetAddress(target: LocalMcpTarget): string {
  const t = target as Record<string, unknown>;
  if (t["sse"]) {
    const sse = t["sse"] as { host: string; port?: number; path?: string };
    return `${sse.host}${sse.port ? `:${sse.port}` : ""}${sse.path ?? ""}`;
  }
  if (t["mcp"]) {
    const mcp = t["mcp"] as { host: string; port?: number; path?: string };
    return `${mcp.host}${mcp.port ? `:${mcp.port}` : ""}${mcp.path ?? ""}`;
  }
  if (t["stdio"]) {
    const stdio = t["stdio"] as { cmd: string };
    return stdio.cmd;
  }
  if (t["openapi"]) {
    const oa = t["openapi"] as { host: string; port?: number };
    return `${oa.host}${oa.port ? `:${oa.port}` : ""}`;
  }
  return "—";
}

const TYPE_COLORS: Record<string, string> = {
  SSE: "cyan",
  MCP: "blue",
  STDIO: "purple",
  OpenAPI: "geekblue",
};

export const MCPOverviewPage = () => {
  const navigate = useNavigate();
  const { data: mcp, isLoading } = useMCPConfig();

  if (isLoading) {
    return (
      <Container>
        <PageTitle>MCP Overview</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
        </div>
      </Container>
    );
  }

  const targets = mcp?.targets ?? [];

  return (
    <Container>
      <PageHeader>
        <div>
          <PageTitle>MCP Overview</PageTitle>
          <PageSubtitle>Model Context Protocol gateway configuration</PageSubtitle>
        </div>
        <Button type="primary" onClick={() => navigate("/mcp/servers")}>
          Manage Servers
        </Button>
      </PageHeader>

      {/* Stats */}
      <Row gutter={[16, 16]}>
        <Col xs={12} sm={8}>
          <StatCard>
            <IconLabel>
              <Server size={16} />
              MCP Targets
            </IconLabel>
            <Statistic
              value={targets.length}
              valueStyle={{ color: "var(--color-primary)", fontSize: 28 }}
            />
          </StatCard>
        </Col>
        <Col xs={12} sm={8}>
          <StatCard>
            <IconLabel>
              <Network size={16} />
              Port
            </IconLabel>
            <Statistic
              value={mcp?.port ?? "—"}
              valueStyle={{
                color: mcp?.port ? "var(--color-primary)" : "var(--color-text-tertiary)",
                fontSize: 28,
              }}
            />
          </StatCard>
        </Col>
        <Col xs={12} sm={8}>
          <StatCard>
            <IconLabel>
              <Workflow size={16} />
              Mode
            </IconLabel>
            <Statistic
              value={mcp?.statefulMode ?? "—"}
              valueStyle={{
                color: mcp?.statefulMode ? "var(--color-primary)" : "var(--color-text-tertiary)",
                fontSize: 18,
              }}
            />
          </StatCard>
        </Col>
      </Row>

      {/* Targets Preview */}
      <Card
        title="MCP Targets"
        extra={
          <Button size="small" onClick={() => navigate("/mcp/servers")}>
            View all
          </Button>
        }
      >
        {targets.length === 0 ? (
          <Empty
            description="No MCP targets configured"
            image={Empty.PRESENTED_IMAGE_SIMPLE}
          >
            <Button type="primary" onClick={() => navigate("/mcp/servers")}>
              Add Target
            </Button>
          </Empty>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            {targets.slice(0, 5).map((target, idx) => {
              const type = getTargetType(target);
              const address = getTargetAddress(target);
              return (
                <div
                  key={idx}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 12,
                    padding: "8px 0",
                    borderBottom:
                      idx < Math.min(targets.length, 5) - 1
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
                    <Server size={18} />
                  </div>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontWeight: 500 }}>{target.name}</div>
                    <div
                      style={{
                        fontSize: 12,
                        color: "var(--color-text-secondary)",
                        fontFamily: "monospace",
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                    >
                      {address}
                    </div>
                  </div>
                  <Tag color={TYPE_COLORS[type] ?? "default"}>{type}</Tag>
                </div>
              );
            })}
            {targets.length > 5 && (
              <div
                style={{
                  textAlign: "center",
                  color: "var(--color-text-secondary)",
                  fontSize: 13,
                  paddingTop: 8,
                }}
              >
                +{targets.length - 5} more targets
              </div>
            )}
          </div>
        )}
      </Card>

      {/* Config summary */}
      {mcp && (
        <Card title="Configuration">
          <div style={{ display: "flex", flexWrap: "wrap", gap: 24 }}>
            {mcp.port && (
              <div>
                <div style={{ fontSize: 12, color: "var(--color-text-secondary)", marginBottom: 4 }}>Port</div>
                <Tag>{mcp.port}</Tag>
              </div>
            )}
            {mcp.statefulMode && (
              <div>
                <div style={{ fontSize: 12, color: "var(--color-text-secondary)", marginBottom: 4 }}>Stateful Mode</div>
                <Tag color="blue">{mcp.statefulMode}</Tag>
              </div>
            )}
            {mcp.prefixMode && (
              <div>
                <div style={{ fontSize: 12, color: "var(--color-text-secondary)", marginBottom: 4 }}>Prefix Mode</div>
                <Tag color="cyan">{mcp.prefixMode}</Tag>
              </div>
            )}
            {mcp.policies && (
              <div>
                <div style={{ fontSize: 12, color: "var(--color-text-secondary)", marginBottom: 4 }}>Policies</div>
                <Tag color="green">Configured</Tag>
              </div>
            )}
          </div>
        </Card>
      )}
    </Container>
  );
};
