import styled from "@emotion/styled";
import { Card, Col, Descriptions, Empty, Row, Spin, Tag } from "antd";
import { Server } from "lucide-react";
import { useState } from "react";
import { useMCPConfig } from "../../api";
import type { LocalMcpTarget } from "../../api/types";

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

const TargetCard = styled(Card)<{ selected?: boolean }>`
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
    const stdio = t["stdio"] as { cmd: string; args?: string[] };
    return `${stdio.cmd}${stdio.args ? " " + stdio.args.join(" ") : ""}`;
  }
  if (t["openapi"]) {
    const oa = t["openapi"] as { host: string; port?: number; path?: string };
    return `${oa.host}${oa.port ? `:${oa.port}` : ""}${oa.path ?? ""}`;
  }
  return "—";
}

const TYPE_COLORS: Record<string, string> = {
  SSE: "cyan",
  MCP: "blue",
  STDIO: "purple",
  OpenAPI: "geekblue",
};

function TargetDetailPanel({ target }: { target: LocalMcpTarget }) {
  const type = getTargetType(target);
  const t = target as Record<string, unknown>;

  const baseItems = [
    { key: "name", label: "Name", children: target.name },
    {
      key: "type",
      label: "Type",
      children: (
        <Tag color={TYPE_COLORS[type] ?? "default"}>{type}</Tag>
      ),
    },
    {
      key: "policies",
      label: "Policies",
      children: target.policies ? (
        <Tag color="green">Configured</Tag>
      ) : (
        <span style={{ color: "var(--color-text-tertiary)" }}>None</span>
      ),
    },
  ];

  const connectionItems: { key: string; label: string; children: React.ReactNode }[] = [];

  if (t["sse"]) {
    const sse = t["sse"] as { host: string; port?: number; path?: string };
    connectionItems.push(
      { key: "host", label: "Host", children: <code>{sse.host}</code> },
      ...(sse.port ? [{ key: "port", label: "Port", children: sse.port }] : []),
      ...(sse.path ? [{ key: "path", label: "Path", children: <code>{sse.path}</code> }] : []),
    );
  } else if (t["mcp"]) {
    const mcp = t["mcp"] as { host: string; port?: number; path?: string };
    connectionItems.push(
      { key: "host", label: "Host", children: <code>{mcp.host}</code> },
      ...(mcp.port ? [{ key: "port", label: "Port", children: mcp.port }] : []),
      ...(mcp.path ? [{ key: "path", label: "Path", children: <code>{mcp.path}</code> }] : []),
    );
  } else if (t["stdio"]) {
    const stdio = t["stdio"] as { cmd: string; args?: string[]; env?: Record<string, string> };
    connectionItems.push(
      { key: "cmd", label: "Command", children: <code>{stdio.cmd}</code> },
      ...(stdio.args?.length
        ? [{ key: "args", label: "Arguments", children: <code>{stdio.args.join(" ")}</code> }]
        : []),
      ...(stdio.env && Object.keys(stdio.env).length > 0
        ? [
            {
              key: "env",
              label: "Environment",
              children: (
                <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                  {Object.keys(stdio.env).map((k) => (
                    <Tag key={k} bordered={false}>
                      {k}
                    </Tag>
                  ))}
                </div>
              ),
            },
          ]
        : []),
    );
  } else if (t["openapi"]) {
    const oa = t["openapi"] as { host: string; port?: number; path?: string };
    connectionItems.push(
      { key: "host", label: "Host", children: <code>{oa.host}</code> },
      ...(oa.port ? [{ key: "port", label: "Port", children: oa.port }] : []),
      ...(oa.path ? [{ key: "path", label: "Path", children: <code>{oa.path}</code> }] : []),
    );
  }

  return (
    <Card title={`Target: ${target.name}`} variant="borderless">
      <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
        <Descriptions bordered column={1} size="small" items={baseItems} />
        {connectionItems.length > 0 && (
          <>
            <div
              style={{
                fontSize: 12,
                fontWeight: 600,
                textTransform: "uppercase",
                letterSpacing: "0.05em",
                color: "var(--color-text-secondary)",
              }}
            >
              Connection
            </div>
            <Descriptions bordered column={1} size="small" items={connectionItems} />
          </>
        )}
      </div>
    </Card>
  );
}

export const MCPServersPage = () => {
  const { data: mcp, isLoading } = useMCPConfig();
  const [selectedIdx, setSelectedIdx] = useState<number | null>(null);

  if (isLoading) {
    return (
      <Container>
        <PageTitle>MCP Servers</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
        </div>
      </Container>
    );
  }

  const targets = mcp?.targets ?? [];
  const selectedTarget = selectedIdx !== null ? targets[selectedIdx] : null;

  return (
    <Container>
      <div>
        <PageTitle>MCP Servers</PageTitle>
        <PageSubtitle>
          {targets.length > 0
            ? `${targets.length} target${targets.length !== 1 ? "s" : ""} configured`
            : "No MCP targets configured yet"}
        </PageSubtitle>
      </div>

      {targets.length === 0 ? (
        <Card>
          <Empty
            description={
              <span>
                No MCP targets configured.{" "}
                <span style={{ color: "var(--color-text-secondary)" }}>
                  Add targets to your config file under the <code>mcp.targets</code> key.
                </span>
              </span>
            }
            image={Empty.PRESENTED_IMAGE_SIMPLE}
          />
        </Card>
      ) : (
        <Row gutter={[16, 16]}>
          <Col xs={24} lg={10}>
            <Card title="Targets">
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {targets.map((target, idx) => {
                  const type = getTargetType(target);
                  const address = getTargetAddress(target);
                  return (
                    <TargetCard
                      key={idx}
                      selected={selectedIdx === idx}
                      size="small"
                      onClick={() =>
                        setSelectedIdx(idx === selectedIdx ? null : idx)
                      }
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
                          <Server size={16} />
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
                            {target.name}
                          </div>
                          <div
                            style={{
                              fontSize: 11,
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
                        <Tag
                          color={TYPE_COLORS[type] ?? "default"}
                          style={{ flexShrink: 0 }}
                        >
                          {type}
                        </Tag>
                      </div>
                    </TargetCard>
                  );
                })}
              </div>
            </Card>
          </Col>

          <Col xs={24} lg={14}>
            {selectedTarget ? (
              <TargetDetailPanel target={selectedTarget} />
            ) : (
              <Card variant="borderless">
                <Empty
                  description="Select a target to view details"
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                />
              </Card>
            )}
          </Col>
        </Row>
      )}
    </Container>
  );
};
