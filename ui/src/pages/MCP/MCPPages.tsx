import styled from "@emotion/styled";
import { Client as McpClient } from "@modelcontextprotocol/sdk/client/index.js";
import { SSEClientTransport as McpSseTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import {
  McpError,
  ListToolsResultSchema as McpListToolsResultSchema,
  type ClientRequest as McpClientRequest,
  type Request as McpRequest,
  type Result as McpResult,
  type Tool as McpTool,
} from "@modelcontextprotocol/sdk/types.js";
import { Button, Card, Col, Input, Row, Spin, Tag } from "antd";
import {
  BarChart3,
  ChevronDown,
  ChevronUp,
  FileCode,
  FileText,
  Send,
  Settings,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import toast from "react-hot-toast";
import { z } from "zod";
import { useConfig } from "../../api/hooks";
import { MonacoEditorWithSettings } from "../../components/MonacoEditor";
import { ActionPanel } from "../../components/playground/ActionPanel";
import { CapabilitiesList } from "../../components/playground/CapabilitiesList";
import type { LocalBind, LocalListener, LocalRoute } from "../../config";
import { useTheme } from "../../contexts/ThemeContext";

// Schema for MCP tool invocation response
const McpToolResponseSchema = z.any();

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

const EmptyStateCard = styled(Card)`
  text-align: center;
  .ant-card-body {
    padding: 64px 32px;
  }
`;

const EmptyIcon = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 16px;
  background: var(--color-bg-hover);
  color: var(--color-text-tertiary);
  margin: 0 auto 16px;
`;

// ---------------------------------------------------------------------------
// MCP Logs
// ---------------------------------------------------------------------------

export const MCPLogsPage = () => (
  <Container>
    <PageTitle>MCP Logs</PageTitle>
    <EmptyStateCard>
      <EmptyIcon>
        <FileText size={28} />
      </EmptyIcon>
      <h3 style={{ margin: "0 0 8px", fontSize: 18, fontWeight: 600 }}>
        MCP Request Logs
      </h3>
      <p
        style={{
          margin: "0 0 24px",
          color: "var(--color-text-secondary)",
          maxWidth: 400,
          marginLeft: "auto",
          marginRight: "auto",
        }}
      >
        MCP tool call logs, request and response details, latency, and error
        traces will be displayed here.
      </p>
      <Tag
        bordered={false}
        color="processing"
        style={{ padding: "4px 12px", fontSize: 13 }}
      >
        Coming soon
      </Tag>
    </EmptyStateCard>
  </Container>
);

// ---------------------------------------------------------------------------
// MCP Metrics
// ---------------------------------------------------------------------------

export const MCPMetricsPage = () => (
  <Container>
    <PageTitle>MCP Metrics</PageTitle>
    <EmptyStateCard>
      <EmptyIcon>
        <BarChart3 size={28} />
      </EmptyIcon>
      <h3 style={{ margin: "0 0 8px", fontSize: 18, fontWeight: 600 }}>
        MCP Performance Metrics
      </h3>
      <p
        style={{
          margin: "0 0 24px",
          color: "var(--color-text-secondary)",
          maxWidth: 400,
          marginLeft: "auto",
          marginRight: "auto",
        }}
      >
        Tool call counts, latency distributions, error rates, and per-target
        analytics will be available here.
      </p>
      <Tag
        bordered={false}
        color="processing"
        style={{ padding: "4px 12px", fontSize: 13 }}
      >
        Coming soon
      </Tag>
    </EmptyStateCard>
  </Container>
);

// ---------------------------------------------------------------------------
// MCP Playground
// ---------------------------------------------------------------------------

const RequestContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
`;

const ResultCard = styled(Card)`
  .ant-card-body {
    padding: 0;
  }
`;

const ResultHeader = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--spacing-md) var(--spacing-lg);
  border-bottom: 1px solid var(--color-border-secondary);
  background: var(--color-bg-container);
`;

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

const RouteCard = styled(Card)`
  cursor: pointer;
  transition: all 0.15s ease;
  position: relative;

  &::before {
    content: "";
    position: absolute;
    inset: 0;
    background: var(--color-primary);
    opacity: 0;
    transition: opacity 0.15s ease;
    pointer-events: none;
    border-radius: inherit;
  }

  &:hover {
    border-color: var(--color-primary);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.12);

    &::before {
      opacity: 0.03;
    }
  }

  &:active {
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.08);

    &::before {
      opacity: 0.05;
    }
  }
`;

const ExpandButton = styled.button`
  position: absolute;
  bottom: 0;
  left: 50%;
  transform: translateX(-50%);
  background: transparent;
  border: none;
  padding: 2px 8px;
  cursor: pointer;
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 11px;
  color: var(--color-text-secondary);
  opacity: 0.7;
  transition: all 0.2s;

  &:hover {
    opacity: 1;
    color: var(--color-text-base);
  }

  svg {
    width: 12px;
    height: 12px;
  }
`;

// Interface for route testing
interface RouteInfo {
  bindPort: number;
  listener: LocalListener;
  route: LocalRoute;
  endpoint: string;
  protocol: string;
  routeIndex: number;
  routePath: string;
}

// Define state interfaces for MCP
interface ConnectionState {
  authToken: string;
  isConnected: boolean;
  isConnecting: boolean;
}

interface McpState {
  client: McpClient<McpRequest, any, McpResult> | null;
  tools: McpTool[];
  selectedTool: McpTool | null;
  paramValues: Record<string, any>;
  response: any;
}

interface UiState {
  isRequestRunning: boolean;
  isLoadingCapabilities: boolean;
}

export const MCPPlaygroundPage = () => {
  const { data: config, isLoading: configLoading } = useConfig();
  const { theme } = useTheme();
  const [routes, setRoutes] = useState<RouteInfo[]>([]);
  const [selectedRoute, setSelectedRoute] = useState<RouteInfo | null>(null);
  const [resultExpanded, setResultExpanded] = useState<boolean>(true);

  // MCP connection state
  const [connectionState, setConnectionState] = useState<ConnectionState>({
    authToken: "",
    isConnected: false,
    isConnecting: false,
  });

  const [mcpState, setMcpState] = useState<McpState>({
    client: null,
    tools: [],
    selectedTool: null,
    paramValues: {},
    response: null,
  });

  const [uiState, setUiState] = useState<UiState>({
    isRequestRunning: false,
    isLoadingCapabilities: false,
  });

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

  const handleRouteSelect = useCallback((routeInfo: RouteInfo) => {
    setSelectedRoute(routeInfo);
    // Reset client state when changing routes
    setConnectionState((prev) => ({
      ...prev,
      isConnected: false,
    }));
    setMcpState({
      client: null,
      tools: [],
      selectedTool: null,
      paramValues: {},
      response: null,
    });
  }, []);

  const handleAuthTokenChange = (token: string) => {
    setConnectionState((prev) => ({ ...prev, authToken: token }));
  };

  // MCP connection function
  const connect = async () => {
    if (!selectedRoute) return;

    setConnectionState((prev) => ({ ...prev, isConnecting: true }));

    try {
      const client = new McpClient(
        { name: "agentgateway-dashboard", version: "0.1.0" },
        { capabilities: {} },
      );

      const headers: Record<string, string> = {
        Accept: "text/event-stream",
        "Cache-Control": "no-cache",
        "mcp-protocol-version": "2024-11-05",
      };

      if (connectionState.authToken && connectionState.authToken.trim()) {
        headers["Authorization"] = `Bearer ${connectionState.authToken}`;
      }

      const sseUrl = selectedRoute.endpoint.endsWith("/")
        ? `${selectedRoute.endpoint}sse`
        : `${selectedRoute.endpoint}/sse`;
      const transport = new McpSseTransport(new URL(sseUrl), {
        eventSourceInit: {
          fetch: (url, init) => {
            return fetch(url, {
              ...init,
              headers: headers as HeadersInit,
            });
          },
        },
        requestInit: {
          headers: headers as HeadersInit,
          credentials: "omit",
          mode: "cors",
        },
      });

      await client.connect(transport);
      setMcpState((prev) => ({ ...prev, client }));
      setConnectionState((prev) => ({ ...prev, isConnected: true }));
      toast.success("Connected to MCP endpoint");

      setUiState((prev) => ({ ...prev, isLoadingCapabilities: true }));
      const listToolsRequest: McpClientRequest = {
        method: "tools/list",
        params: {},
      };
      const toolsResponse = await client.request(
        listToolsRequest,
        McpListToolsResultSchema,
      );
      setMcpState((prev) => ({ ...prev, tools: toolsResponse.tools }));
      setUiState((prev) => ({ ...prev, isLoadingCapabilities: false }));
    } catch (error: any) {
      console.error("Connection failed:", error);
      let errorMessage = "Failed to connect";
      if (error.message?.includes("CORS")) {
        errorMessage =
          "CORS error: Check if the endpoint allows requests from this origin";
      } else if (error.message) {
        errorMessage = error.message;
      }

      toast.error(errorMessage);
      setConnectionState((prev) => ({
        ...prev,
        isConnected: false,
        isConnecting: false,
      }));
      setUiState((prev) => ({ ...prev, isLoadingCapabilities: false }));
    } finally {
      setConnectionState((prev) => ({ ...prev, isConnecting: false }));
    }
  };

  const runMcpTool = async () => {
    if (!mcpState.client || !mcpState.selectedTool) return;

    setUiState((prev) => ({ ...prev, isRequestRunning: true }));
    setMcpState((prev) => ({ ...prev, response: null }));

    try {
      const request: McpClientRequest = {
        method: "tools/call",
        params: {
          name: mcpState.selectedTool.name,
          arguments: mcpState.paramValues,
        },
      };
      const result = await mcpState.client.request(
        request,
        McpToolResponseSchema,
      );
      setMcpState((prev) => ({ ...prev, response: result }));
      toast.success(`Tool ${mcpState.selectedTool?.name} executed.`);
    } catch (error: any) {
      const message =
        error instanceof McpError ? error.message : "Failed to run tool";
      setMcpState((prev) => ({
        ...prev,
        response: { error: message, details: error },
      }));
      toast.error(message);
    } finally {
      setUiState((prev) => ({ ...prev, isRequestRunning: false }));
    }
  };

  const handleMcpToolSelect = (tool: McpTool) => {
    setMcpState((prev) => ({ ...prev, selectedTool: tool, response: null }));

    // Initialize parameter values based on tool schema
    const initialParams: Record<string, any> = {};
    if (tool.inputSchema?.properties) {
      Object.keys(tool.inputSchema.properties).forEach((key) => {
        const prop = tool.inputSchema!.properties![key] as any;
        switch (prop?.type) {
          case "string":
            initialParams[key] = "";
            break;
          case "number":
            initialParams[key] = 0;
            break;
          case "boolean":
            initialParams[key] = false;
            break;
          case "array":
            initialParams[key] = [];
            break;
          case "object":
            initialParams[key] = {};
            break;
          default:
            initialParams[key] = "";
        }
      });
    }
    setMcpState((prev) => ({ ...prev, paramValues: initialParams }));
  };

  const handleMcpParamChange = (paramName: string, value: any) => {
    setMcpState((prev) => ({
      ...prev,
      paramValues: { ...prev.paramValues, [paramName]: value },
    }));
  };

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
        {/* Route Selection */}
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "1rem",
            marginBottom: "1rem",
          }}
        >
          {routes.map((routeInfo, idx) => {
            const isSelected = selectedRoute === routeInfo;
            return (
              <RouteCard
                key={`${routeInfo.bindPort}-${routeInfo.routeIndex}`}
                size="small"
                style={{
                  background: isSelected
                    ? "var(--color-bg-selected)"
                    : "var(--color-bg-spotlight)",
                  border: isSelected
                    ? "2px solid var(--color-primary)"
                    : undefined,
                }}
                onClick={() => handleRouteSelect(routeInfo)}
              >
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "0.5rem",
                  }}
                >
                  <span style={{ fontWeight: 500 }}>
                    {routeInfo.route.name || `Route ${idx + 1}`}
                  </span>
                  <Tag color="blue">Port {routeInfo.bindPort}</Tag>
                  <Tag style={{ fontSize: "11px", fontFamily: "monospace" }}>
                    {routeInfo.routePath}
                  </Tag>
                  <Tag color="purple">MCP</Tag>
                  <span
                    style={{
                      marginLeft: "auto",
                      fontSize: "12px",
                      color: "var(--color-text-secondary)",
                      fontFamily: "monospace",
                    }}
                  >
                    {routeInfo.endpoint}
                  </span>
                </div>
              </RouteCard>
            );
          })}
        </div>

        {/* Auth Token and Connect Button */}
        {selectedRoute && (
          <div
            style={{ display: "flex", gap: "1rem", alignItems: "flex-start" }}
          >
            <div style={{ flex: 1 }}>
              <label
                style={{
                  display: "block",
                  marginBottom: "8px",
                  fontSize: "14px",
                }}
              >
                Auth Token (optional)
              </label>
              <Input
                placeholder="Bearer token for authentication"
                value={connectionState.authToken}
                onChange={(e) => handleAuthTokenChange(e.target.value)}
              />
            </div>
            <div style={{ paddingTop: "30px" }}>
              <Button
                type="primary"
                onClick={connect}
                loading={connectionState.isConnecting}
                disabled={!selectedRoute}
              >
                {connectionState.isConnected ? "Reconnect" : "Connect to MCP"}
              </Button>
            </div>
          </div>
        )}
      </SectionCard>

      {/* Tools and Testing Section */}
      {selectedRoute && connectionState.isConnected && (
        <Row gutter={[16, 16]}>
          {/* Left Column: Available Tools */}
          <Col xs={24} lg={8}>
            <CapabilitiesList
              connectionType="mcp"
              isLoading={uiState.isLoadingCapabilities}
              mcpTools={mcpState.tools}
              a2aSkills={[]}
              a2aAgentCard={null}
              selectedMcpToolName={mcpState.selectedTool?.name || null}
              selectedA2aSkillId={null}
              onMcpToolSelect={handleMcpToolSelect}
              onA2aSkillSelect={() => {}}
            />
          </Col>

          {/* Right Column: Request and Response */}
          <Col xs={24} lg={16}>
            <div
              style={{ display: "flex", flexDirection: "column", gap: "16px" }}
            >
              {/* Top: User Request */}
              <SectionCard
                title={
                  <>
                    <Send size={18} /> Request
                  </>
                }
              >
                <ActionPanel
                  connectionType="mcp"
                  mcpSelectedTool={mcpState.selectedTool}
                  a2aSelectedSkill={null}
                  mcpParamValues={mcpState.paramValues}
                  a2aMessage=""
                  isRequestRunning={uiState.isRequestRunning}
                  onMcpParamChange={handleMcpParamChange}
                  onA2aMessageChange={() => {}}
                  onRunMcpTool={runMcpTool}
                  onRunA2aSkill={() => {}}
                />
              </SectionCard>

              {/* Bottom: Response */}
              <div style={{ position: "relative" }}>
                <SectionCard
                  title={
                    <>
                      <FileCode size={18} /> Response
                    </>
                  }
                >
                  {mcpState.response ? (
                    <div style={{ padding: 0 }}>
                      <MonacoEditorWithSettings
                        value={JSON.stringify(mcpState.response, null, 2)}
                        language="json"
                        height={resultExpanded ? "300px" : "70px"}
                        theme={theme}
                        readOnly
                        options={{
                          readOnly: true,
                          minimap: { enabled: false },
                          scrollBeyondLastLine: false,
                        }}
                      />
                    </div>
                  ) : (
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        justifyContent: "center",
                        height: "70px",
                        color: "var(--color-text-secondary)",
                        fontSize: "14px",
                      }}
                    >
                      Select a tool and click "Run Tool" to see the response
                    </div>
                  )}
                </SectionCard>
                {mcpState.response && (
                  <ExpandButton
                    type="button"
                    onClick={() => setResultExpanded(!resultExpanded)}
                  >
                    {resultExpanded ? (
                      <>
                        <ChevronUp size={14} />
                        Collapse
                      </>
                    ) : (
                      <>
                        <ChevronDown size={14} />
                        Expand
                      </>
                    )}
                  </ExpandButton>
                )}
              </div>
            </div>
          </Col>
        </Row>
      )}
    </Container>
  );
};
