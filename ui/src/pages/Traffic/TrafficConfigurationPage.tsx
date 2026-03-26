import { CodeOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Spin } from "antd";
import { Network } from "lucide-react";
import { useMemo, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { StyledAlert } from "../../components/StyledAlert";
import type { AddRootHandlers, UrlParams } from "../../components/TrafficHierarchy";
import {
  HierarchyTree,
  NodeDetailView,
  useTrafficHierarchy,
} from "../../components/TrafficHierarchy";

// ---------------------------------------------------------------------------
// Styled components
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// URL parsing — extract hierarchy position from the current pathname
// ---------------------------------------------------------------------------

function parseTrafficPath(pathname: string): UrlParams | null {
  // Check for model routes first (must be before general LLM route)
  const modelMatch = pathname.match(/\/traffic\/llm\/model\/(\d+)/);
  if (modelMatch) {
    return {
      topLevelType: "llm",
      modelIndex: parseInt(modelMatch[1], 10),
    };
  }

  // Check for LLM policy routes (must be before general LLM route)
  const llmPolicyMatch = pathname.match(/\/traffic\/llm\/policy\/(.+)/);
  if (llmPolicyMatch) {
    return {
      topLevelType: "llm",
      llmPolicyType: llmPolicyMatch[1],
    };
  }

  // Check for MCP policy routes (must be before general MCP route)
  const mcpPolicyMatch = pathname.match(/\/traffic\/mcp\/policy\/(.+)/);
  if (mcpPolicyMatch) {
    return {
      topLevelType: "mcp",
      mcpPolicyType: mcpPolicyMatch[1],
    };
  }

  // Check for MCP target policy routes (must be before target route)
  const mcpTargetPolicyMatch = pathname.match(/\/traffic\/mcp\/target\/(\d+)\/policy\/(.+)/);
  if (mcpTargetPolicyMatch) {
    return {
      topLevelType: "mcp",
      mcpTargetIndex: parseInt(mcpTargetPolicyMatch[1], 10),
      mcpTargetPolicyType: mcpTargetPolicyMatch[2],
    };
  }

  // Check for MCP target routes (must be before general MCP route)
  const mcpTargetMatch = pathname.match(/\/traffic\/mcp\/target\/(\d+)/);
  if (mcpTargetMatch) {
    return {
      topLevelType: "mcp",
      mcpTargetIndex: parseInt(mcpTargetMatch[1], 10),
    };
  }

  // Check for top-level config routes
  const topLevelMatch = pathname.match(/\/traffic\/(llm|mcp|frontendPolicies)/);
  if (topLevelMatch) {
    return {
      topLevelType: topLevelMatch[1] as "llm" | "mcp" | "frontendPolicies",
    };
  }

  // Check for bind routes
  const m = pathname.match(
    /\/traffic\/bind\/(\d+)(?:\/listener\/(\d+)(?:\/(tcp)?route\/(\d+)(?:\/backend\/(\d+)|\/policy\/([^/?]+))?)?)?/,
  );
  if (!m) return null;

  const bi = m[5] !== undefined ? parseInt(m[5], 10) : undefined;
  const policyType = m[6]; // Policy type like 'cors', 'requestHeaderModifier', etc.

  return {
    port: parseInt(m[1], 10),
    li: m[2] !== undefined ? parseInt(m[2], 10) : undefined,
    isTcpRoute: m[3] === "tcp",
    ri: m[4] !== undefined ? parseInt(m[4], 10) : undefined,
    bi,
    policyType,
  };
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export function TrafficConfigurationPage() {
  const hierarchy = useTrafficHierarchy();
  const location = useLocation();
  const navigate = useNavigate();
  const [addHandlers, setAddHandlers] = useState<AddRootHandlers | null>(null);

  const urlParams = useMemo(
    () => parseTrafficPath(location.pathname),
    [location.pathname],
  );

  if (hierarchy.error) {
    return (
      <PageRoot>
        <MetricsHeader>
          <PageHeader>
            <div>
              <PageTitle>Traffic Configuration</PageTitle>
              <Description>
                View and edit the full agentgateway configuration.
              </Description>
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
              <PageTitle>Traffic Configuration</PageTitle>
              <Description>
                View and edit the full agentgateway configuration.
              </Description>
            </div>
          </PageHeader>
        </MetricsHeader>
        <div style={{ textAlign: "center", padding: 50, flex: 1 }}>
          <Spin size="large" />
          <div style={{ marginTop: 16, color: "var(--color-text-secondary)" }}>
            Loading traffic configuration…
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
            <PageTitle>Traffic Configuration</PageTitle>
            <Description>
              View and edit the full agentgateway configuration.
            </Description>
          </div>
          <Button
            icon={<CodeOutlined />}
            onClick={() => navigate("/traffic/raw-config")}
          >
            Config Editor
          </Button>
        </PageHeader>
      </MetricsHeader>
      <SplitBody>
        <Sidebar>
          <HierarchyTree hierarchy={hierarchy} onRegisterAddHandlers={setAddHandlers} />
        </Sidebar>
        <DetailPanel>
          {shouldShowDetail ? (
            <NodeDetailView hierarchy={hierarchy} urlParams={urlParams} />
          ) : (
            <PlaceholderContainer>
              <PlaceholderContent>
                <h3>Traffic Configuration</h3>
                {hierarchy.binds.length > 0 ? (
                  <p>
                    Choose a bind, listener, route, backend, or policy from the
                    hierarchy tree on the left to view and edit its configuration.
                  </p>
                ) : (
                  <>
                    <p>
                      No binds configured. Create a bind to start defining
                      listeners, routes, and backends.
                    </p>
                    <div style={{ marginTop: 16 }}>
                      <Button
                        type="primary"
                        icon={<Network size={16} />}
                        onClick={() => addHandlers?.addBind()}
                        disabled={!addHandlers}
                      >
                        Add Bind
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
