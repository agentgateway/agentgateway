import { CodeOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Spin } from "antd";
import { useMemo } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { StyledAlert } from "../../components/StyledAlert";
import {
  HierarchyTree,
  NodeDetailView,
  useTrafficHierarchy,
} from "../../components/TrafficHierarchy";
import type { UrlParams } from "../../components/TrafficHierarchy";

// ---------------------------------------------------------------------------
// Styled components (shared layout with TrafficPage / LLMPage)
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
// Base path for MCP pages is /mcp, so hierarchy URL is /mcp/mcp
// ---------------------------------------------------------------------------

function parseMCPPath(pathname: string): UrlParams | null {
  if (pathname.startsWith("/mcp/mcp")) {
    return { topLevelType: "mcp" };
  }
  return null;
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export function MCPPage() {
  const hierarchy = useTrafficHierarchy();
  const location = useLocation();
  const navigate = useNavigate();

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
            onClick={() => navigate("/mcp/raw-config")}
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
          />
        </Sidebar>
        <DetailPanel>
          {shouldShowDetail ? (
            <NodeDetailView hierarchy={hierarchy} urlParams={urlParams} />
          ) : (
            <PlaceholderContainer>
              <PlaceholderContent>
                <h3>Select an Item</h3>
                <p>
                  Choose the MCP configuration from the tree on the left to
                  view and edit its settings.
                </p>
              </PlaceholderContent>
            </PlaceholderContainer>
          )}
        </DetailPanel>
      </SplitBody>
    </PageRoot>
  );
}
