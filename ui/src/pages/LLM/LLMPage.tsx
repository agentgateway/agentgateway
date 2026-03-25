import { CodeOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Spin } from "antd";
import { Boxes, Shield } from "lucide-react";
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
// Styled components (shared with TrafficPage layout)
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

// ---------------------------------------------------------------------------
// URL parsing — extract hierarchy position from the current pathname
// Base path for LLM pages is /llm, so hierarchy URLs are /llm/llm, /llm/llm/model/:idx
// ---------------------------------------------------------------------------

function parseLLMPath(pathname: string): UrlParams | null {
  const modelMatch = pathname.match(/^\/llm\/llm\/model\/(\d+)/);
  if (modelMatch) {
    return { topLevelType: "llm", modelIndex: parseInt(modelMatch[1], 10) };
  }
  if (pathname.startsWith("/llm/llm")) {
    return { topLevelType: "llm" };
  }
  return null;
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

export function LLMPage() {
  const hierarchy = useTrafficHierarchy();
  const location = useLocation();
  const navigate = useNavigate();

  const urlParams = useMemo(
    () => parseLLMPath(location.pathname),
    [location.pathname],
  );

  if (hierarchy.error) {
    return (
      <PageRoot>
        <MetricsHeader>
          <PageHeader>
            <div>
              <PageTitle>LLM Configuration</PageTitle>
              <Description>Manage LLM models and policies</Description>
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
              <PageTitle>LLM Configuration</PageTitle>
              <Description>Manage LLM models and policies</Description>
            </div>
          </PageHeader>
        </MetricsHeader>
        <div style={{ textAlign: "center", padding: 50, flex: 1 }}>
          <Spin size="large" />
          <div style={{ marginTop: 16, color: "var(--color-text-secondary)" }}>
            Loading LLM configuration…
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
            <PageTitle>LLM Configuration</PageTitle>
            <Description>Manage LLM models and policies</Description>
          </div>
          <Button
            icon={<CodeOutlined />}
            onClick={() => navigate("/llm/raw-config")}
          >
            Config Editor
          </Button>
        </PageHeader>
      </MetricsHeader>
      <SplitBody>
        <Sidebar>
          <HierarchyTree
            hierarchy={hierarchy}
            filter={["llm"]}
            title="LLM Configuration"
          />
        </Sidebar>
        <DetailPanel>
          {shouldShowDetail ? (
            <NodeDetailView hierarchy={hierarchy} urlParams={urlParams} />
          ) : (
            <PlaceholderContainer>
              <PlaceholderContent>
                <h3>LLM Configuration</h3>
                <p>
                  Select an item from the tree, or add LLM configuration to
                  get started.
                </p>
                <PlaceholderIcons>
                  <IconItem>
                    <Boxes size={20} /> Models
                  </IconItem>
                  <IconItem>
                    <Shield size={20} /> Policies
                  </IconItem>
                </PlaceholderIcons>
                <p className="hint">
                  Models and policies added to your agentgateway configuration
                  will appear here.
                </p>
              </PlaceholderContent>
            </PlaceholderContainer>
          )}
        </DetailPanel>
      </SplitBody>
    </PageRoot>
  );
}
