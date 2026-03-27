import styled from "@emotion/styled";
import { Card, Spin } from "antd";
import { Settings } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useConfig } from "../../../api/hooks";
import type { LocalBind, LocalListener, LocalRoute } from "../../../config";
import { RouteSelector } from "./RouteSelector";
import { ToolTester } from "./ToolTester";
import type { RouteInfo } from "./types";
import { useConnection } from "./useConnection";

const SectionCard = styled(Card)`
  .ant-card-head {
    background: var(--color-bg-container);
    border-bottom: 1px solid var(--color-border-secondary);
    padding: var(--spacing-md) var(--spacing-lg);
    min-height: auto;
    display: flex;
    align-items: center;
  }

  .ant-card-head-title {
    font-weight: 600;
    font-size: 15px;
    padding: 0;
    display: flex;
    align-items: center;
    gap: 8px;

    svg {
      flex-shrink: 0;
    }
  }

  .ant-card-body {
    padding: var(--spacing-lg);
  }
`;

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
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

export function MCPPlaygroundPage() {
  const { data: config, isLoading: configLoading } = useConfig();
  const [routes, setRoutes] = useState<RouteInfo[]>([]);
  const [selectedRoute, setSelectedRoute] = useState<RouteInfo | null>(null);
  const [resultExpanded, setResultExpanded] = useState<boolean>(true);

  const {
    connectionState,
    mcpState,
    uiState,
    resetConnectionForRoute,
    handleAuthTokenChange,
    connect,
    runMcpTool,
    handleMcpToolSelect,
    handleMcpParamChange,
  } = useConnection(selectedRoute, routes);

  // Extract routes from configuration that have MCP backends
  useEffect(() => {
    if (!config || !config.binds) return;

    const extractedRoutes: RouteInfo[] = [];

    config.binds.forEach((bind: LocalBind) => {
      bind.listeners.forEach((listener: LocalListener) => {
        if (listener.routes) {
          listener.routes.forEach((route: LocalRoute, routeIndex: number) => {
            // Only include routes with MCP backends
            const hasMcpBackend = route.backends?.some((b: any) => b.mcp);
            if (!hasMcpBackend) return;

            const protocol = listener.protocol === "HTTPS" ? "https" : "http";
            const hostname = listener.hostname || "localhost";
            const port = bind.port;
            const baseEndpoint = `${protocol}://${hostname}:${port}`;

            let routePath = "/";
            if (route.matches?.[0]?.path) {
              const pathMatch = route.matches[0].path;
              if ("exact" in pathMatch) {
                routePath = pathMatch.exact;
              } else if ("pathPrefix" in pathMatch) {
                routePath = pathMatch.pathPrefix;
              }
            }

            extractedRoutes.push({
              bindPort: port,
              listener,
              route,
              endpoint: baseEndpoint,
              protocol,
              routeIndex,
              routePath,
            });
          });
        }
      });
    });

    setRoutes(extractedRoutes);
  }, [config]);

  const handleRouteSelect = useCallback(
    (routeInfo: RouteInfo) => {
      setSelectedRoute(routeInfo);
      resetConnectionForRoute();
    },
    [resetConnectionForRoute],
  );

  if (configLoading) {
    return (
      <Container>
        <PageTitle>MCP Playground</PageTitle>
        <div style={{ textAlign: "center", padding: 60 }}>
          <Spin size="large" />
          <p style={{ marginTop: "1rem" }}>Loading routes...</p>
        </div>
      </Container>
    );
  }

  if (routes.length === 0) {
    return (
      <Container>
        <PageTitle>MCP Playground</PageTitle>
        <PageSubtitle>Test MCP server tool calls interactively</PageSubtitle>
        <Card style={{ marginTop: "1rem" }}>
          <div style={{ textAlign: "center", padding: "2rem" }}>
            <p>
              No routes with MCP backends configured. Please add routes with MCP
              backends to your agentgateway configuration.
            </p>
          </div>
        </Card>
      </Container>
    );
  }

  return (
    <Container>
      <PageTitle>MCP Playground</PageTitle>
      <PageSubtitle>Test MCP server tool calls interactively</PageSubtitle>

      {/* Connection Section */}
      <SectionCard
        title={
          <>
            <Settings size={18} /> Connection
          </>
        }
      >
        <RouteSelector
          routes={routes}
          selectedRoute={selectedRoute}
          connectionState={connectionState}
          onSelectRoute={handleRouteSelect}
          onAuthTokenChange={handleAuthTokenChange}
          onConnect={connect}
        />
      </SectionCard>

      {/* Tools and Testing Section */}
      {selectedRoute && connectionState.isConnected && (
        <ToolTester
          mcpState={mcpState}
          uiState={uiState}
          resultExpanded={resultExpanded}
          onToolSelect={handleMcpToolSelect}
          onParamChange={handleMcpParamChange}
          onRunTool={runMcpTool}
          onToggleExpand={() => setResultExpanded((prev) => !prev)}
        />
      )}
    </Container>
  );
}
