import { CodeOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Spin } from "antd";
import { Headphones, Server, Shield } from "lucide-react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { StyledAlert } from "../../components/StyledAlert";
import type { AddRootHandlers, UrlParams } from "../../components/TrafficHierarchy";
import {
  HierarchyTree,
  NodeDetailView,
  useTrafficHierarchy,
} from "../../components/TrafficHierarchy";

const PageRoot = styled.div`
  display: flex;
  flex-direction: column;
  height: calc(100vh - 64px);
  overflow: hidden;
`;

const MetricsHeader = styled.div`
  padding: var(--spacing-lg) var(--spacing-xl);
  border-bottom: 1px solid var(--color-border);
  background: var(--color-bg-layout);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
`;

const SplitBody = styled.div`
  display: flex;
  flex: 1;
  overflow: hidden;
`;

const Sidebar = styled.div`
  width: 380px;
  flex-shrink: 0;
  overflow-y: auto;
  border-right: 1px solid var(--color-border);
  padding: var(--spacing-lg);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
`;

const DetailPanel = styled.div`
  flex: 1;
  overflow-y: auto;
`;

const PlaceholderContainer = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100%;
  padding: var(--spacing-xl);
`;

const PlaceholderContent = styled.div`
  text-align: center;
  max-width: 400px;

  h3 {
    font-size: 18px;
    font-weight: 600;
    color: var(--color-text-base);
    margin-bottom: var(--spacing-md);
  }

  p {
    color: var(--color-text-secondary);
    font-size: 14px;
    line-height: 1.6;
  }

  .hint {
    font-size: 13px;
    color: var(--color-text-tertiary);
    margin-top: var(--spacing-lg);
  }
`;

const PlaceholderIcons = styled.div`
  display: flex;
  justify-content: center;
  gap: var(--spacing-xl);
  margin-top: var(--spacing-lg);
`;

const IconItem = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  color: var(--color-text-secondary);
  font-size: 14px;
  font-weight: 500;
`;

const PageHeader = styled.div`
  display: flex;
  justify-content: space-between;
  align-items: center;
`;

const PageTitle = styled.h1`
  margin: 0;
  font-size: 24px;
  font-weight: 600;
  color: var(--color-text-base);
`;

const Description = styled.p`
  color: var(--color-text-secondary);
  margin: 0;
  font-size: 14px;
`;

function parseMCPPath(pathname: string): UrlParams | null {
  const targetPolicyMatch = pathname.match(/^\/mcp\/target\/(\d+)\/policy\/(.+)/);
  if (targetPolicyMatch) {
    return {
      topLevelType: "mcp",
      mcpTargetIndex: parseInt(targetPolicyMatch[1], 10),
      mcpTargetPolicyType: targetPolicyMatch[2],
    };
  }
  const targetMatch = pathname.match(/^\/mcp\/target\/(\d+)/);
  if (targetMatch) {
    return { topLevelType: "mcp", mcpTargetIndex: parseInt(targetMatch[1], 10) };
  }
  const policyMatch = pathname.match(/^\/mcp\/policy\/(.+)/);
  if (policyMatch) {
    return { topLevelType: "mcp", mcpPolicyType: policyMatch[1] };
  }
  if (pathname.startsWith("/mcp")) {
    return { topLevelType: "mcp" };
  }
  return null;
}

export function MCPConfigurationPage() {
  const hierarchy = useTrafficHierarchy();
  const location = useLocation();
  const navigate = useNavigate();
  const [addHandlers, setAddHandlers] = useState<AddRootHandlers | null>(null);

  const urlParams = useMemo(
    () => parseMCPPath(location.pathname),
    [location.pathname],
  );

  if (hierarchy.error) {
    return (
      <PageRoot>
        <MetricsHeader>
          <PageHeader>
            <div>
              <PageTitle>MCP Configuration</PageTitle>
              <Description>Manage MCP servers and policies</Description>
            </div>
          </PageHeader>
          <StyledAlert
            message="Error Loading Configuration"
            description={hierarchy.error.message ?? "Failed to load config"}
            type="error"
            showIcon
          />
        </MetricsHeader>
      </PageRoot>
    );
  }

  if (hierarchy.isLoading) {
    return (
      <PageRoot>
        <MetricsHeader>
          <PageHeader>
            <div>
              <PageTitle>MCP Configuration</PageTitle>
              <Description>Manage MCP servers and policies</Description>
            </div>
          </PageHeader>
        </MetricsHeader>
        <div style={{ textAlign: "center", padding: 50, flex: 1 }}>
          <Spin size="large" />
          <div style={{ marginTop: 16, color: "var(--color-text-secondary)" }}>
            Loading MCP configuration…
          </div>
        </div>
      </PageRoot>
    );
  }

  const shouldShowDetail = urlParams !== null;

  return (
    <PageRoot>
      <MetricsHeader>
        <PageHeader>
          <div>
            <PageTitle>MCP Configuration</PageTitle>
            <Description>Manage MCP servers and policies</Description>
          </div>
          <Button
            icon={<CodeOutlined />}
            onClick={() => navigate("/mcp-configuration/editor")}
          >
            Config Editor
          </Button>
        </PageHeader>
      </MetricsHeader>
      <SplitBody>
        <Sidebar>
          <HierarchyTree
            hierarchy={hierarchy}
            filter={["mcp"]}
            title="MCP Configuration"
            onRegisterAddHandlers={setAddHandlers}
          />
        </Sidebar>
        <DetailPanel>
          {shouldShowDetail ? (
            <NodeDetailView hierarchy={hierarchy} urlParams={urlParams} />
          ) : (
            <PlaceholderContainer>
              <PlaceholderContent>
                <h3>MCP Configuration</h3>
                {hierarchy.mcp ? (
                  <>
                    <p>
                      Select an item from the tree to view and edit its
                      configuration.
                    </p>
                    <PlaceholderIcons>
                      <IconItem>
                        <Server size={20} /> Servers
                      </IconItem>
                      <IconItem>
                        <Shield size={20} /> Policies
                      </IconItem>
                    </PlaceholderIcons>
                  </>
                ) : (
                  <>
                    <p>
                      No MCP configuration found. Create one to start
                      configuring servers and policies.
                    </p>
                    <div style={{ marginTop: 16 }}>
                      <Button
                        type="primary"
                        icon={<Headphones size={16} />}
                        onClick={() => addHandlers?.addMCP()}
                        disabled={!addHandlers}
                      >
                        Add MCP Config
                      </Button>
                    </div>
                  </>
                )}
              </PlaceholderContent>
            </PlaceholderContainer>
          )}
        </DetailPanel>
      </SplitBody>
    </PageRoot>
  );
}
