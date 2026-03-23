import { CodeOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Spin } from "antd";
import { useMemo } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { StyledAlert } from "../../components/StyledAlert";
import { HierarchyTree } from "./components/HierarchyTree";
import { NodeDetailView } from "./components/NodeDetailView";
import { useTrafficHierarchy } from "./hooks/useTrafficHierarchy";

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

export interface UrlParams {
  port?: number;
  li?: number;
  isTcpRoute?: boolean;
  ri?: number;
  bi?: number;
  policyType?: string;
  topLevelType?: "llm" | "mcp" | "frontendPolicies";
  modelIndex?: number;
}

function parseTrafficPath(pathname: string): UrlParams | null {
  // Check for model routes first (must be before general LLM route)
  const modelMatch = pathname.match(/\/traffic\/llm\/model\/(\d+)/);
  if (modelMatch) {
    return {
      topLevelType: "llm",
      modelIndex: parseInt(modelMatch[1], 10),
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

export function TrafficPage() {
  const hierarchy = useTrafficHierarchy();
  const location = useLocation();
  const navigate = useNavigate();

  // Parse URL to determine if we're viewing a specific node
  const urlParams = useMemo(
    () => parseTrafficPath(location.pathname),
    [location.pathname],
  );

  // ---------------------------------------------------------------------------
  // Error / loading states
  // ---------------------------------------------------------------------------
  if (hierarchy.error) {
    return (
      <PageRoot>
        <MetricsHeader>
          <PageHeader>
            <div>
              <PageTitle>Traffic Configuration (Manual Schemas)</PageTitle>
              <Description>
                Manage your gateway routing with manually configured TypeScript
                schemas
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
              <PageTitle>Traffic Configuration (Manual Schemas)</PageTitle>
              <Description>
                Manage your gateway routing with manually configured TypeScript
                schemas
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

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  // Determine if we should show a detail view or placeholder
  // Show detail if we have a bind (port) or any sub-resource selected
  const shouldShowDetail = urlParams !== null;

  return (
    <PageRoot>
      <MetricsHeader>
        <PageHeader>
          <div>
            <PageTitle>Traffic Configuration (Manual Schemas)</PageTitle>
            <Description>
              Manage your gateway routing with manually configured TypeScript
              schemas
            </Description>
          </div>
          <Button
            icon={<CodeOutlined />}
            onClick={() => navigate("/traffic/raw-config")}
          >
            Config Editor
          </Button>
        </PageHeader>
        <StyledAlert
          message="Manual TypeScript Schemas"
          description="This page uses manually configured TypeScript form schemas (not auto-generated JSON). Forms are defined in traffic/forms/ and use config.d.ts types directly for compile-time safety."
          type="info"
          showIcon
          closable
        />
      </MetricsHeader>
      <SplitBody>
        <Sidebar>
          <HierarchyTree hierarchy={hierarchy} />
        </Sidebar>
        <DetailPanel>
          {shouldShowDetail ? (
            <NodeDetailView hierarchy={hierarchy} urlParams={urlParams} />
          ) : (
            <PlaceholderContainer>
              <PlaceholderContent>
                <h3>Select an Item</h3>
                <p>
                  Choose a bind, listener, route, backend, or policy from the
                  hierarchy tree on the left to view and edit its configuration.
                </p>
              </PlaceholderContent>
            </PlaceholderContainer>
          )}
        </DetailPanel>
      </SplitBody>
    </PageRoot>
  );
}
