import type {
  AgentCard,
  AgentSkill,
  Message,
  MessageSendParams,
  Task,
} from "@a2a-js/sdk";
import {
  A2AClient,
  createAuthenticatingFetchWithRetry,
  type AuthenticationHandler,
} from "@a2a-js/sdk/client";
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
import {
  Button,
  Card,
  Form,
  Input,
  message,
  Select,
  Spin,
  Tabs,
  Tag,
} from "antd";
import { useCallback, useEffect, useState } from "react";
import toast from "react-hot-toast";
import { v4 as uuidv4 } from "uuid";
import { z } from "zod";
import { useConfig } from "../../api/hooks";
import { ActionPanel } from "../../components/playground/ActionPanel";
import { CapabilitiesList } from "../../components/playground/CapabilitiesList";
import { ResponseDisplay } from "../../components/playground/ResponseDisplay";
import type {
  LocalBind,
  LocalListener,
  LocalRoute,
  LocalRouteBackend,
} from "../../config";

// Schema for MCP tool invocation response
const McpToolResponseSchema = z.any();

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  padding: var(--spacing-xl);
`;

const RequestContainer = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
`;

const CodeBlock = styled.pre`
  background: var(--color-bg-elevated);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-base);
  padding: var(--spacing-md);
  overflow: auto;
  max-height: 400px;
  font-family: "Monaco", "Consolas", monospace;
  font-size: 13px;
`;

const StatusBadge = styled.div<{ success?: boolean }>`
  display: inline-block;
  padding: 4px 12px;
  border-radius: var(--border-radius-base);
  background: ${(props) =>
    props.success ? "var(--color-success-bg)" : "var(--color-error-bg)"};
  color: ${(props) =>
    props.success ? "var(--color-success)" : "var(--color-error)"};
  font-weight: 500;
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
  routeDescription: string;
}

interface TestRequest {
  method: string;
  path: string;
  headers: Record<string, string>;
  body: string;
  query: Record<string, string>;
}

interface TestResponse {
  status: number;
  statusText: string;
  headers: Record<string, string>;
  body: string;
  responseTime: number;
  timestamp: string;
}

// Define state interfaces for MCP/A2A
interface ConnectionState {
  selectedEndpoint: string;
  selectedListenerName: string | null;
  selectedListenerProtocol: string | null;
  authToken: string;
  connectionType: "mcp" | "a2a" | "http" | null;
  isConnected: boolean;
  isConnecting: boolean;
  isLoadingA2aTargets: boolean;
}

interface McpState {
  client: McpClient<McpRequest, any, McpResult> | null;
  tools: McpTool[];
  selectedTool: McpTool | null;
  paramValues: Record<string, any>;
  response: any;
}

interface A2aState {
  client: A2AClient | null;
  targets: string[];
  selectedTarget: string | null;
  agentCard: AgentCard | null;
  skills: AgentSkill[];
  selectedSkill: AgentSkill | null;
  message: string;
  response: Task | any | null;
}

interface UiState {
  isRequestRunning: boolean;
  isLoadingCapabilities: boolean;
}

const HTTP_METHODS = [
  "GET",
  "POST",
  "PUT",
  "DELETE",
  "PATCH",
  "HEAD",
  "OPTIONS",
];

export const PlaygroundPage = () => {
  const { data: config, isLoading: configLoading } = useConfig();
  const [routes, setRoutes] = useState<RouteInfo[]>([]);
  const [selectedRoute, setSelectedRoute] = useState<RouteInfo | null>(null);

  // HTTP testing state
  const [request, setRequest] = useState<TestRequest>({
    method: "GET",
    path: "/",
    headers: {},
    body: "",
    query: {},
  });
  const [response, setResponse] = useState<TestResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [headerKey, setHeaderKey] = useState("");
  const [headerValue, setHeaderValue] = useState("");
  const [queryKey, setQueryKey] = useState("");
  const [queryValue, setQueryValue] = useState("");

  // MCP/A2A connection state
  const [connectionState, setConnectionState] = useState<ConnectionState>({
    selectedEndpoint: "",
    selectedListenerName: null,
    selectedListenerProtocol: null,
    authToken: "",
    connectionType: null,
    isConnected: false,
    isConnecting: false,
    isLoadingA2aTargets: false,
  });

  const [mcpState, setMcpState] = useState<McpState>({
    client: null,
    tools: [],
    selectedTool: null,
    paramValues: {},
    response: null,
  });

  const [a2aState, setA2aState] = useState<A2aState>({
    client: null,
    targets: [],
    selectedTarget: null,
    agentCard: null,
    skills: [],
    selectedSkill: null,
    message: "",
    response: null,
  });

  const [uiState, setUiState] = useState<UiState>({
    isRequestRunning: false,
    isLoadingCapabilities: false,
  });

  const [_form] = Form.useForm();

  // Extract routes from configuration
  useEffect(() => {
    if (!config || !config.binds) return;

    const extractedRoutes: RouteInfo[] = [];

    config.binds.forEach((bind: LocalBind) => {
      bind.listeners.forEach(
        (listener: LocalListener, _listenerIndex: number) => {
          if (listener.routes) {
            listener.routes.forEach((route: LocalRoute, routeIndex: number) => {
              const protocol = listener.protocol === "HTTPS" ? "https" : "http";
              const hostname = listener.hostname || "localhost";
              const port = bind.port;
              const baseEndpoint = `${protocol}://${hostname}:${port}`;

              // Generate route path and description with better pattern recognition
              let routePath = "/";
              let routePattern = "/*";

              if (route.matches?.[0]?.path) {
                const pathMatch = route.matches[0].path;
                if ("exact" in pathMatch) {
                  routePath = pathMatch.exact;
                  routePattern = pathMatch.exact;
                } else if ("pathPrefix" in pathMatch) {
                  routePath = pathMatch.pathPrefix;
                  routePattern = pathMatch.pathPrefix + "*";
                } else if ("regex" in pathMatch) {
                  routePath = "/";
                  routePattern = `~${pathMatch.regex}`;
                }
              }

              // Create full endpoint with route path
              const endpoint = `${baseEndpoint}${routePath}`;

              const hostnames = route.hostnames?.join(", ") || "";
              const backendCount = route.backends?.length || 0;
              const backendTypes =
                route.backends
                  ?.map((b) => {
                    if ((b as any).mcp) return "MCP";
                    if ((b as any).ai) return "AI";
                    if ((b as any).service) return "Service";
                    if ((b as any).host) return "Host";
                    if ((b as any).dynamic) return "Dynamic";
                    return "Unknown";
                  })
                  .join(", ") || "";

              const routeDescription = `${routePattern}${hostnames ? ` • ${hostnames}` : ""} • ${backendCount} backend${backendCount !== 1 ? "s" : ""}${backendTypes ? ` (${backendTypes})` : ""}`;

              extractedRoutes.push({
                bindPort: bind.port,
                listener,
                route,
                endpoint,
                protocol,
                routeIndex,
                routePath: routePattern,
                routeDescription,
              });
            });
          }
        },
      );
    });

    setRoutes(extractedRoutes);

    // Auto-select first route if available
    if (extractedRoutes.length > 0 && !selectedRoute) {
      setSelectedRoute(extractedRoutes[0]);
      updateRequestFromRoute(extractedRoutes[0]);
    }
  }, [config]);

  // Determine backend type of selected route
  const getRouteBackendType = (route: RouteInfo): "mcp" | "a2a" | "http" => {
    // Check if route has A2A policy first - this takes precedence
    if (route.route.policies?.a2a) {
      return "a2a";
    }

    if (!route.route.backends || route.route.backends.length === 0)
      return "http";

    const backend = route.route.backends[0];
    if ((backend as any).mcp) return "mcp";
    return "http"; // AI, Host, Service, etc.
  };

  const updateRequestFromRoute = useCallback((routeInfo: RouteInfo) => {
    let initialPath = "/";

    if (routeInfo.route.matches && routeInfo.route.matches.length > 0) {
      const firstMatch = routeInfo.route.matches[0];
      if (firstMatch.path && "exact" in firstMatch.path) {
        initialPath = firstMatch.path.exact;
      } else if (firstMatch.path && "pathPrefix" in firstMatch.path) {
        initialPath = firstMatch.path.pathPrefix;
      } else if (firstMatch.path && "regex" in firstMatch.path) {
        initialPath = "/";
      }
    }

    setRequest({
      method: "GET",
      path: initialPath,
      headers: {},
      body: "",
      query: {},
    });
    setResponse(null);

    // Reset MCP/A2A responses
    setMcpState((prev) => ({ ...prev, response: null }));
    setA2aState((prev) => ({ ...prev, response: null }));

    // Set connection type based on backend
    const backendType = getRouteBackendType(routeInfo);
    setConnectionState((prev) => ({
      ...prev,
      connectionType: backendType,
      selectedEndpoint: routeInfo.endpoint,
      selectedListenerName: routeInfo.listener.name || null,
      selectedListenerProtocol: routeInfo.listener.protocol || null,
    }));
  }, []);

  const resetClientState = () => {
    setConnectionState((prev) => ({
      ...prev,
      connectionType: connectionState.connectionType,
    }));
    setMcpState((prev) => ({
      ...prev,
      client: null,
      tools: [],
      selectedTool: null,
      paramValues: {},
      response: null,
    }));
    setA2aState((prev) => ({
      ...prev,
      client: null,
      agentCard: null,
      skills: [],
      selectedSkill: null,
      message: "",
      response: null,
    }));
    setUiState({
      isLoadingCapabilities: false,
      isRequestRunning: false,
    });
  };

  // MCP/A2A connection functions
  const connect = async () => {
    if (!selectedRoute) return;

    setConnectionState((prev) => ({ ...prev, isConnecting: true }));
    resetClientState();

    const backendType = getRouteBackendType(selectedRoute);

    try {
      if (backendType === "mcp") {
        setConnectionState((prev) => ({ ...prev, connectionType: "mcp" }));

        // TODO: Support acting as a stateless client
        const client = new McpClient(
          { name: "agentgateway-dashboard", version: "0.1.0" },
          { capabilities: {} },
        );

        const headers: Record<string, string> = {
          Accept: "text/event-stream",
          "Cache-Control": "no-cache",
          "mcp-protocol-version": "2024-11-05",
        };

        // Only add auth header if token is provided and not empty
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
      } else if (backendType === "a2a") {
        // Connect to A2A endpoint
        setConnectionState((prev) => ({ ...prev, connectionType: "a2a" }));
        const connectUrl = selectedRoute.endpoint;

        let client: A2AClient;

        if (connectionState.authToken && connectionState.authToken.trim()) {
          // Create authentication handler for bearer token
          const authHandler: AuthenticationHandler = {
            headers: async () => ({
              Authorization: `Bearer ${connectionState.authToken}`,
            }),
            shouldRetryWithHeaders: async (
              _req: RequestInit,
              res: Response,
            ) => {
              // Retry with auth headers on 401 or 403 responses
              if (res.status === 401 || res.status === 403) {
                return {
                  Authorization: `Bearer ${connectionState.authToken}`,
                };
              }
              return undefined;
            },
          };

          const authenticatedFetch = createAuthenticatingFetchWithRetry(
            fetch,
            authHandler,
          );
          client = new A2AClient(connectUrl, {
            fetchImpl: authenticatedFetch,
          });
        } else {
          client = new A2AClient(connectUrl);
        }

        setA2aState((prev) => ({ ...prev, client }));
        setConnectionState((prev) => ({ ...prev, isConnected: true }));
        toast.success("Connected to A2A endpoint");

        // Load A2A capabilities
        setUiState((prev) => ({ ...prev, isLoadingCapabilities: true }));
        try {
          // Fetch the agent card using the client's built-in method
          // The client already handles authentication via the fetchImpl we provided
          const agentCard: AgentCard = await client.getAgentCard();

          // Extract skills from the agent card
          const skills = agentCard.skills || [];

          setA2aState((prev) => ({ ...prev, agentCard, skills }));
          toast.success(
            `Loaded A2A agent: ${agentCard.name} with ${skills.length} skill${skills.length !== 1 ? "s" : ""}`,
          );
        } catch (error: any) {
          console.error("Failed to load A2A capabilities:", error);
          // Don't fail the connection, just continue without skills
          setA2aState((prev) => ({ ...prev, skills: [] }));

          // Provide specific guidance for CORS errors
          let errorMessage = "Unknown error loading agent card";
          if (error.message?.includes("CORS")) {
            errorMessage =
              "CORS error: Check if the A2A endpoint allows requests from this origin";
          } else if (
            error.message?.includes("401") ||
            error.message?.includes("403")
          ) {
            errorMessage = "Authentication error: Check your auth token";
          } else if (error.message) {
            errorMessage = error.message;
          }

          toast.error(`Failed to load agent card: ${errorMessage}`);
        }
      }

      setUiState((prev) => ({ ...prev, isLoadingCapabilities: false }));
    } catch (error: any) {
      console.error("Connection failed:", error);
      let errorMessage = "Failed to connect";
      if (error.message?.includes("CORS")) {
        errorMessage =
          "CORS error: Check if the endpoint allows requests from this origin";
      } else if (
        error.message?.includes("401") ||
        error.message?.includes("403")
      ) {
        errorMessage = "Authentication error: Check your auth token";
      } else if (error.message) {
        errorMessage = error.message;
      }

      toast.error(errorMessage);
      resetClientState();
    } finally {
      setConnectionState((prev) => ({ ...prev, isConnecting: false }));
    }
  };

  const runMcpTool = async () => {
    if (!mcpState.client || !mcpState.selectedTool) return;

    setUiState((prev) => ({ ...prev, isRequestRunning: true }));
    setMcpState((prev) => ({ ...prev, response: null }));
    setA2aState((prev) => ({ ...prev, response: null }));

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

  const runA2aSkill = async () => {
    if (
      !a2aState.client ||
      !a2aState.selectedSkill ||
      !a2aState.message.trim()
    ) {
      if (!a2aState.message.trim())
        toast.error("Please enter a message for the agent.");
      return;
    }

    setUiState((prev) => ({ ...prev, isRequestRunning: true }));
    setA2aState((prev) => ({ ...prev, response: null }));
    setMcpState((prev) => ({ ...prev, response: null }));

    try {
      const message: Message = {
        role: "user",
        parts: [{ kind: "text", text: a2aState.message }],
        kind: "message",
        messageId: uuidv4(),
      };

      const params: MessageSendParams = {
        message: message,
      };

      const taskResult = await a2aState.client.sendMessage(params);
      setA2aState((prev) => ({ ...prev, response: taskResult }));
      toast.success(
        `Task sent to agent using skill ${a2aState.selectedSkill?.name}.`,
      );
    } catch (error: any) {
      console.error("Failed to run A2A skill:", error);
      const message =
        error instanceof Error
          ? `Error: ${error.message}`
          : "Failed to send task";
      setA2aState((prev) => ({
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
    setA2aState((prev) => ({ ...prev, response: null }));

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

  const handleA2aSkillSelect = (skill: AgentSkill) => {
    setA2aState((prev) => ({
      ...prev,
      selectedSkill: skill,
      response: null,
      message: "",
    }));
    setMcpState((prev) => ({ ...prev, response: null }));
  };

  const handleMcpParamChange = (key: string, value: any) => {
    setMcpState((prev) => ({
      ...prev,
      paramValues: { ...prev.paramValues, [key]: value },
    }));
  };

  const handleAuthTokenChange = (token: string) => {
    setConnectionState((prev) => ({ ...prev, authToken: token }));
  };

  const handleA2aMessageChange = (message: string) => {
    setA2aState((prev) => ({ ...prev, message }));
  };

  const handleRouteSelect = (routeInfo: RouteInfo) => {
    // Don't allow selection of routes with no backends unless they have A2A policy
    const hasBackends =
      routeInfo.route.backends && routeInfo.route.backends.length > 0;
    const hasA2aPolicy = routeInfo.route.policies?.a2a;

    if (!hasBackends && !hasA2aPolicy) {
      message.error(
        "Cannot test route without backends or A2A policy. Please configure at least one backend or enable A2A policy for this route.",
      );
      return;
    }

    setSelectedRoute(routeInfo);
    updateRequestFromRoute(routeInfo);
  };

  // HTTP request functions
  const addHeader = () => {
    if (headerKey && headerValue) {
      setRequest((prev) => ({
        ...prev,
        headers: { ...prev.headers, [headerKey]: headerValue },
      }));
      setHeaderKey("");
      setHeaderValue("");
    }
  };

  const removeHeader = (key: string) => {
    setRequest((prev) => ({
      ...prev,
      headers: Object.fromEntries(
        Object.entries(prev.headers).filter(([k]) => k !== key),
      ),
    }));
  };

  const addQuery = () => {
    if (queryKey && queryValue) {
      setRequest((prev) => ({
        ...prev,
        query: { ...prev.query, [queryKey]: queryValue },
      }));
      setQueryKey("");
      setQueryValue("");
    }
  };

  const removeQuery = (key: string) => {
    setRequest((prev) => ({
      ...prev,
      query: Object.fromEntries(
        Object.entries(prev.query).filter(([k]) => k !== key),
      ),
    }));
  };

  const sendHttpRequest = async () => {
    if (!selectedRoute) return;

    setIsLoading(true);
    const startTime = performance.now();

    try {
      const url = new URL(selectedRoute.endpoint + request.path);

      // Add query parameters
      Object.entries(request.query).forEach(([key, value]) => {
        url.searchParams.append(key, value);
      });

      const fetchOptions: RequestInit = {
        method: request.method,
        headers: {
          "Content-Type": "application/json",
          ...request.headers,
        },
      };

      if (request.body && ["POST", "PUT", "PATCH"].includes(request.method)) {
        fetchOptions.body = request.body;
      }

      const res = await fetch(url.toString(), fetchOptions);
      const endTime = performance.now();
      const responseTime = endTime - startTime;

      const responseBody = await res.text();
      const responseHeaders: Record<string, string> = {};
      res.headers.forEach((value, key) => {
        responseHeaders[key] = value;
      });

      setResponse({
        status: res.status,
        statusText: res.statusText,
        headers: responseHeaders,
        body: responseBody,
        responseTime,
        timestamp: new Date().toISOString(),
      });

      toast.success(`Request completed in ${responseTime.toFixed(2)}ms`);
    } catch (error) {
      const endTime = performance.now();
      const responseTime = endTime - startTime;

      setResponse({
        status: 0,
        statusText: "Network Error",
        headers: {},
        body: error instanceof Error ? error.message : "Unknown error",
        responseTime,
        timestamp: new Date().toISOString(),
      });

      toast.error("Request failed");
    } finally {
      setIsLoading(false);
    }
  };

  const HttpRequestTab = () => (
    <RequestContainer>
      {configLoading ? (
        <div style={{ textAlign: "center", padding: "2rem" }}>
          <Spin size="large" />
          <p style={{ marginTop: "1rem" }}>Loading configuration...</p>
        </div>
      ) : !selectedRoute ? (
        <div style={{ textAlign: "center", padding: "2rem" }}>
          <p>
            No routes available. Please configure routes in your agentgateway
            configuration.
          </p>
        </div>
      ) : (
        <>
          {/* Request URL Display */}
          <Card size="small" style={{ marginBottom: "1rem" }}>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "0.5rem",
                marginBottom: "0.5rem",
              }}
            >
              <span style={{ fontWeight: 500 }}>Request URL</span>
            </div>
            <div
              style={{
                fontFamily: "monospace",
                fontSize: "13px",
                wordBreak: "break-all",
              }}
            >
              {selectedRoute.protocol}://
              {selectedRoute.listener.hostname || "localhost"}:
              {selectedRoute.bindPort}
              {request.path}
            </div>
          </Card>

          {/* Method and Path Configuration */}
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "150px 1fr",
              gap: "1rem",
              marginBottom: "1rem",
            }}
          >
            <div>
              <label
                style={{
                  display: "block",
                  marginBottom: "0.5rem",
                  fontWeight: 500,
                }}
              >
                HTTP Method
              </label>
              <Select
                value={request.method}
                onChange={(value) =>
                  setRequest((prev) => ({ ...prev, method: value }))
                }
                style={{ width: "100%" }}
              >
                {HTTP_METHODS.map((method) => (
                  <Select.Option key={method} value={method}>
                    {method}
                  </Select.Option>
                ))}
              </Select>
            </div>

            <div>
              <label
                style={{
                  display: "block",
                  marginBottom: "0.5rem",
                  fontWeight: 500,
                }}
              >
                Request Path
                {selectedRoute.route.matches?.[0]?.path &&
                  "regex" in selectedRoute.route.matches[0].path && (
                    <span
                      style={{
                        fontSize: "12px",
                        color: "var(--color-text-secondary)",
                        marginLeft: "0.5rem",
                      }}
                    >
                      (Must match pattern:{" "}
                      {Array.isArray(selectedRoute.route.matches[0].path.regex)
                        ? selectedRoute.route.matches[0].path.regex.join(", ")
                        : selectedRoute.route.matches[0].path.regex}
                      )
                    </span>
                  )}
                {selectedRoute.route.matches?.[0]?.path &&
                  "pathPrefix" in selectedRoute.route.matches[0].path && (
                    <span
                      style={{
                        fontSize: "12px",
                        color: "var(--color-text-secondary)",
                        marginLeft: "0.5rem",
                      }}
                    >
                      (Must start with:{" "}
                      {selectedRoute.route.matches[0].path.pathPrefix})
                    </span>
                  )}
                {selectedRoute.route.matches?.[0]?.path &&
                  "exact" in selectedRoute.route.matches[0].path && (
                    <span
                      style={{
                        fontSize: "12px",
                        color: "var(--color-text-secondary)",
                        marginLeft: "0.5rem",
                      }}
                    >
                      (Must be exactly:{" "}
                      {selectedRoute.route.matches[0].path.exact})
                    </span>
                  )}
              </label>
              <div style={{ display: "flex", gap: "0.5rem" }}>
                <Input
                  value={request.path}
                  onChange={(e) =>
                    setRequest((prev) => ({ ...prev, path: e.target.value }))
                  }
                  placeholder={
                    selectedRoute.route.matches?.[0]?.path &&
                    "regex" in selectedRoute.route.matches[0].path
                      ? "/your/path/here"
                      : selectedRoute.route.matches?.[0]?.path &&
                          "pathPrefix" in selectedRoute.route.matches[0].path
                        ? `${selectedRoute.route.matches[0].path.pathPrefix}...`
                        : selectedRoute.route.matches?.[0]?.path &&
                            "exact" in selectedRoute.route.matches[0].path
                          ? selectedRoute.route.matches[0].path.exact
                          : "/path"
                  }
                  style={{ flex: 1 }}
                />
                <Button
                  type="primary"
                  onClick={sendHttpRequest}
                  loading={isLoading}
                >
                  Send
                </Button>
              </div>
            </div>
          </div>

          {/* Route Info */}
          <Card size="small" style={{ marginBottom: "1rem" }}>
            <h4 style={{ margin: "0 0 0.5rem 0", fontWeight: 500 }}>
              Route Configuration
            </h4>
            <div style={{ display: "grid", gap: "0.5rem", fontSize: "14px" }}>
              <div style={{ display: "flex", justifyContent: "space-between" }}>
                <span style={{ color: "var(--color-text-secondary)" }}>
                  Name:
                </span>
                <span>{selectedRoute.route.name || "Unnamed"}</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between" }}>
                <span style={{ color: "var(--color-text-secondary)" }}>
                  Listener:
                </span>
                <span>{selectedRoute.listener.name || "Unnamed"}</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between" }}>
                <span style={{ color: "var(--color-text-secondary)" }}>
                  Port:
                </span>
                <span>{selectedRoute.bindPort}</span>
              </div>
              <div style={{ display: "flex", justifyContent: "space-between" }}>
                <span style={{ color: "var(--color-text-secondary)" }}>
                  Route Pattern:
                </span>
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "0.5rem",
                  }}
                >
                  <span style={{ fontFamily: "monospace", fontSize: "12px" }}>
                    {selectedRoute.routePath}
                  </span>
                  {selectedRoute.route.matches?.[0]?.path &&
                    "regex" in selectedRoute.route.matches[0].path && (
                      <Tag color="blue">regex</Tag>
                    )}
                  {selectedRoute.route.matches?.[0]?.path &&
                    "pathPrefix" in selectedRoute.route.matches[0].path && (
                      <Tag color="green">prefix</Tag>
                    )}
                  {selectedRoute.route.matches?.[0]?.path &&
                    "exact" in selectedRoute.route.matches[0].path && (
                      <Tag color="orange">exact</Tag>
                    )}
                </div>
              </div>
              {selectedRoute.route.hostnames &&
                selectedRoute.route.hostnames.length > 0 && (
                  <div
                    style={{ display: "flex", justifyContent: "space-between" }}
                  >
                    <span style={{ color: "var(--color-text-secondary)" }}>
                      Host Match:
                    </span>
                    <span style={{ fontSize: "12px" }}>
                      {selectedRoute.route.hostnames.join(", ")}
                    </span>
                  </div>
                )}
              <div>
                <span style={{ color: "var(--color-text-secondary)" }}>
                  Backends:
                </span>
                <div style={{ marginTop: "0.25rem" }}>
                  {selectedRoute.route.backends?.map((backend, idx) => {
                    const getBackendInfo = (backend: LocalRouteBackend) => {
                      if ((backend as any).mcp) {
                        return { type: "MCP", name: "MCP Backend" };
                      } else if ((backend as any).host) {
                        return {
                          type: "Host",
                          name:
                            (backend as any).host.Hostname?.[0] ||
                            (backend as any).host.Address ||
                            "Unknown",
                        };
                      } else if ((backend as any).service) {
                        return {
                          type: "Service",
                          name: (backend as any).service.name.hostname,
                        };
                      } else if ((backend as any).ai) {
                        return { type: "AI", name: (backend as any).ai.name };
                      }
                      return { type: "Unknown", name: "Unknown" };
                    };

                    const info = getBackendInfo(backend);
                    return (
                      <div
                        key={idx}
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: "0.5rem",
                          fontSize: "12px",
                        }}
                      >
                        <Tag>{info.type}</Tag>
                        <span>{info.name}</span>
                        {(backend as any).weight &&
                          (backend as any).weight !== 1 && (
                            <span
                              style={{ color: "var(--color-text-secondary)" }}
                            >
                              (weight: {(backend as any).weight})
                            </span>
                          )}
                      </div>
                    );
                  }) || (
                    <div
                      style={{
                        fontSize: "12px",
                        color: "var(--color-text-secondary)",
                      }}
                    >
                      No backends configured
                    </div>
                  )}
                </div>
              </div>
            </div>
          </Card>

          <Tabs defaultValue="headers" type="card" size="small">
            <Tabs.TabPane tab="Headers" key="headers">
              <div
                style={{ display: "flex", gap: "0.5rem", marginBottom: "1rem" }}
              >
                <Input
                  placeholder="Header name"
                  value={headerKey}
                  onChange={(e) => setHeaderKey(e.target.value)}
                />
                <Input
                  placeholder="Header value"
                  value={headerValue}
                  onChange={(e) => setHeaderValue(e.target.value)}
                />
                <Button onClick={addHeader}>Add</Button>
              </div>
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: "0.5rem",
                }}
              >
                {Object.entries(request.headers).map(([key, value]) => (
                  <div
                    key={key}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      padding: "0.5rem",
                      background: "var(--color-bg-hover)",
                      borderRadius: "4px",
                    }}
                  >
                    <span style={{ fontSize: "14px" }}>
                      <span style={{ fontWeight: 500 }}>{key}:</span> {value}
                    </span>
                    <Button size="small" onClick={() => removeHeader(key)}>
                      Remove
                    </Button>
                  </div>
                ))}
              </div>
            </Tabs.TabPane>

            <Tabs.TabPane tab="Query" key="query">
              <div
                style={{ display: "flex", gap: "0.5rem", marginBottom: "1rem" }}
              >
                <Input
                  placeholder="Query parameter name"
                  value={queryKey}
                  onChange={(e) => setQueryKey(e.target.value)}
                />
                <Input
                  placeholder="Query parameter value"
                  value={queryValue}
                  onChange={(e) => setQueryValue(e.target.value)}
                />
                <Button onClick={addQuery}>Add</Button>
              </div>
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: "0.5rem",
                }}
              >
                {Object.entries(request.query).map(([key, value]) => (
                  <div
                    key={key}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      padding: "0.5rem",
                      background: "var(--color-bg-hover)",
                      borderRadius: "4px",
                    }}
                  >
                    <span style={{ fontSize: "14px" }}>
                      <span style={{ fontWeight: 500 }}>{key}:</span> {value}
                    </span>
                    <Button size="small" onClick={() => removeQuery(key)}>
                      Remove
                    </Button>
                  </div>
                ))}
              </div>
            </Tabs.TabPane>

            <Tabs.TabPane tab="Body" key="body">
              <Input.TextArea
                placeholder="Enter request body (JSON, XML, etc.)"
                value={request.body}
                onChange={(e) =>
                  setRequest((prev) => ({ ...prev, body: e.target.value }))
                }
                rows={6}
                style={{ fontFamily: "monospace" }}
              />
            </Tabs.TabPane>
          </Tabs>
        </>
      )}
    </RequestContainer>
  );

  const RouteTestingTab = () => (
    <RequestContainer>
      <p>
        Test your route configurations here. This tool allows you to verify
        routing rules and backend connections.
      </p>
      <Form layout="vertical">
        <Form.Item label="Route Path" required>
          <Input placeholder="/api/v1/chat" />
        </Form.Item>
        <Form.Item label="Test Request">
          <Input.TextArea
            rows={6}
            placeholder='{"method": "POST", "headers": {}, "body": {}}'
          />
        </Form.Item>
        <Button type="primary">Test Route</Button>
      </Form>
    </RequestContainer>
  );

  const MCPClientTab = () => (
    <RequestContainer>
      <p>
        Test MCP (Model Context Protocol) server connections and tool calls.
      </p>

      {/* Connection Controls */}
      <div style={{ marginBottom: "1rem" }}>
        <Form.Item label="Auth Token (optional)">
          <Input
            placeholder="Bearer token for authentication"
            value={connectionState.authToken}
            onChange={(e) => handleAuthTokenChange(e.target.value)}
          />
        </Form.Item>
        <Button
          type="primary"
          onClick={connect}
          loading={connectionState.isConnecting}
          disabled={!selectedRoute || connectionState.connectionType !== "mcp"}
        >
          {connectionState.isConnected ? "Reconnect" : "Connect to MCP"}
        </Button>
      </div>

      {/* Capabilities Display */}
      {connectionState.isConnected &&
        connectionState.connectionType === "mcp" && (
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
        )}

      {/* Action Panel */}
      {connectionState.isConnected &&
        connectionState.connectionType === "mcp" && (
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
        )}

      {/* Response Display */}
      {mcpState.response && (
        <ResponseDisplay
          connectionType="mcp"
          mcpResponse={mcpState.response}
          a2aResponse={null}
        />
      )}
    </RequestContainer>
  );

  const A2AClientTab = () => (
    <RequestContainer>
      <p>Test Agent-to-Agent (A2A) connections and skill execution.</p>

      {/* Connection Controls */}
      <div style={{ marginBottom: "1rem" }}>
        <Form.Item label="Auth Token (optional)">
          <Input
            placeholder="Bearer token for authentication"
            value={connectionState.authToken}
            onChange={(e) => handleAuthTokenChange(e.target.value)}
          />
        </Form.Item>
        <Button
          type="primary"
          onClick={connect}
          loading={connectionState.isConnecting}
          disabled={!selectedRoute || connectionState.connectionType !== "a2a"}
        >
          {connectionState.isConnected ? "Reconnect" : "Connect to A2A"}
        </Button>
      </div>

      {/* Capabilities Display */}
      {connectionState.isConnected &&
        connectionState.connectionType === "a2a" && (
          <CapabilitiesList
            connectionType="a2a"
            isLoading={uiState.isLoadingCapabilities}
            mcpTools={[]}
            a2aSkills={a2aState.skills}
            a2aAgentCard={a2aState.agentCard}
            selectedMcpToolName={null}
            selectedA2aSkillId={a2aState.selectedSkill?.id || null}
            onMcpToolSelect={() => {}}
            onA2aSkillSelect={handleA2aSkillSelect}
          />
        )}

      {/* Action Panel */}
      {connectionState.isConnected &&
        connectionState.connectionType === "a2a" && (
          <ActionPanel
            connectionType="a2a"
            mcpSelectedTool={null}
            a2aSelectedSkill={a2aState.selectedSkill}
            mcpParamValues={{}}
            a2aMessage={a2aState.message}
            isRequestRunning={uiState.isRequestRunning}
            onMcpParamChange={() => {}}
            onA2aMessageChange={handleA2aMessageChange}
            onRunMcpTool={() => {}}
            onRunA2aSkill={runA2aSkill}
          />
        )}

      {/* Response Display */}
      {a2aState.response && (
        <ResponseDisplay
          connectionType="a2a"
          mcpResponse={null}
          a2aResponse={a2aState.response}
        />
      )}
    </RequestContainer>
  );

  const RouteSelection = () => {
    if (configLoading) {
      return (
        <Card style={{ marginBottom: "1rem" }}>
          <div style={{ textAlign: "center", padding: "2rem" }}>
            <Spin size="large" />
            <p style={{ marginTop: "1rem" }}>Loading routes...</p>
          </div>
        </Card>
      );
    }

    if (routes.length === 0) {
      return (
        <Card style={{ marginBottom: "1rem" }}>
          <div style={{ textAlign: "center", padding: "2rem" }}>
            <p>
              No routes configured. Please add routes to your agentgateway
              configuration.
            </p>
          </div>
        </Card>
      );
    }

    // Group routes by bind port and listener
    const groupedRoutes = new Map<string, RouteInfo[]>();
    routes.forEach((routeInfo) => {
      const groupKey = `${routeInfo.bindPort}-${routeInfo.listener.name || "unnamed"}`;
      if (!groupedRoutes.has(groupKey)) {
        groupedRoutes.set(groupKey, []);
      }
      groupedRoutes.get(groupKey)!.push(routeInfo);
    });

    return (
      <Card style={{ marginBottom: "1rem" }}>
        <div style={{ display: "flex", flexDirection: "column", gap: "1rem" }}>
          {Array.from(groupedRoutes.entries()).map(([groupKey, routeInfos]) => {
            const firstRoute = routeInfos[0];
            const port = firstRoute.bindPort;
            const listenerName = firstRoute.listener.name || "unnamed";
            const endpoint = firstRoute.endpoint;

            return (
              <Card
                key={groupKey}
                size="small"
                style={{ background: "var(--color-bg-spotlight)" }}
              >
                {/* Group Header */}
                <div
                  style={{
                    padding: "0.75rem",
                    borderBottom: "1px solid var(--color-border-secondary)",
                  }}
                >
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                    }}
                  >
                    <div
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "0.5rem",
                      }}
                    >
                      <span style={{ fontWeight: 500 }}>{listenerName}</span>
                      <Tag color="blue">Port {port}</Tag>
                    </div>
                    <div
                      style={{
                        fontSize: "12px",
                        color: "var(--color-text-secondary)",
                        fontFamily: "monospace",
                      }}
                    >
                      {endpoint}
                    </div>
                  </div>
                </div>

                {/* Routes in this group */}
                <div
                  style={{
                    borderTop: "1px solid var(--color-border-secondary)",
                  }}
                >
                  {routeInfos.map((routeInfo, index) => {
                    const hasBackends =
                      routeInfo.route.backends &&
                      routeInfo.route.backends.length > 0;
                    const hasA2aPolicy = routeInfo.route.policies?.a2a;
                    const backendTypes =
                      routeInfo.route.backends?.map((b) => {
                        if ((b as any).mcp) return "MCP";
                        if ((b as any).ai) return "AI";
                        if ((b as any).service) return "Service";
                        if ((b as any).host) return "Host";
                        if ((b as any).dynamic) return "Dynamic";
                        return "Unknown";
                      }) || [];

                    return (
                      <div
                        key={`${groupKey}-${index}`}
                        style={{
                          padding: "0.75rem",
                          cursor:
                            !hasBackends && !hasA2aPolicy
                              ? "not-allowed"
                              : "pointer",
                          background:
                            selectedRoute === routeInfo
                              ? "var(--color-bg-selected)"
                              : "transparent",
                          borderBottom:
                            index < routeInfos.length - 1
                              ? "1px solid var(--color-border-secondary)"
                              : "none",
                          opacity: !hasBackends && !hasA2aPolicy ? 0.6 : 1,
                        }}
                        onClick={() => handleRouteSelect(routeInfo)}
                      >
                        <div
                          style={{
                            display: "flex",
                            alignItems: "center",
                            justifyContent: "space-between",
                          }}
                        >
                          <div style={{ flex: 1, minWidth: 0 }}>
                            {/* Route name and path */}
                            <div
                              style={{
                                display: "flex",
                                alignItems: "center",
                                gap: "0.5rem",
                                marginBottom: "0.25rem",
                              }}
                            >
                              {!hasBackends && !hasA2aPolicy && (
                                <span style={{ color: "var(--color-error)" }}>
                                  ⚠️
                                </span>
                              )}
                              <span style={{ fontWeight: 500 }}>
                                {routeInfo.route.name ||
                                  `Route ${routeInfo.routeIndex + 1}`}
                              </span>
                              <Tag
                                style={{
                                  fontSize: "11px",
                                  fontFamily: "monospace",
                                }}
                              >
                                {routeInfo.routePath}
                              </Tag>
                              {routeInfo.route.matches?.[0]?.path &&
                                "regex" in routeInfo.route.matches[0].path && (
                                  <Tag color="blue">regex</Tag>
                                )}
                              {routeInfo.route.matches?.[0]?.path &&
                                "pathPrefix" in
                                  routeInfo.route.matches[0].path && (
                                  <Tag color="green">prefix</Tag>
                                )}
                              {routeInfo.route.matches?.[0]?.path &&
                                "exact" in routeInfo.route.matches[0].path && (
                                  <Tag color="orange">exact</Tag>
                                )}
                            </div>

                            {/* Route details */}
                            <div
                              style={{
                                fontSize: "12px",
                                color: "var(--color-text-secondary)",
                                lineHeight: "1.4",
                              }}
                            >
                              {/* Hostnames */}
                              {routeInfo.route.hostnames &&
                                routeInfo.route.hostnames.length > 0 && (
                                  <div>
                                    Hosts:{" "}
                                    {routeInfo.route.hostnames.join(", ")}
                                  </div>
                                )}

                              {/* Backends */}
                              <div
                                style={{
                                  display: "flex",
                                  alignItems: "center",
                                  gap: "0.5rem",
                                  marginTop: "0.25rem",
                                }}
                              >
                                <span>
                                  {hasA2aPolicy && !hasBackends
                                    ? "A2A Traffic"
                                    : `${routeInfo.route.backends?.length || 0} backend${(routeInfo.route.backends?.length || 0) !== 1 ? "s" : ""}`}
                                </span>

                                {/* Backend types and A2A policy */}
                                {(hasBackends || hasA2aPolicy) && (
                                  <div
                                    style={{ display: "flex", gap: "0.25rem" }}
                                  >
                                    {hasA2aPolicy && (
                                      <Tag color="purple">A2A</Tag>
                                    )}
                                    {hasBackends &&
                                      backendTypes.map((type, idx) => (
                                        <Tag key={idx}>{type}</Tag>
                                      ))}
                                  </div>
                                )}
                              </div>

                              {/* Error message */}
                              {!hasBackends && !hasA2aPolicy && (
                                <div
                                  style={{
                                    color: "var(--color-error)",
                                    marginTop: "0.25rem",
                                  }}
                                >
                                  Cannot test - no backends configured
                                </div>
                              )}
                            </div>
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </Card>
            );
          })}
        </div>
      </Card>
    );
  };

  const items = [
    {
      key: "http",
      label: "HTTP Request",
      children: <HttpRequestTab />,
    },
    {
      key: "route",
      label: "Route Testing",
      children: <RouteTestingTab />,
    },
    {
      key: "mcp",
      label: "MCP Client",
      children: <MCPClientTab />,
    },
    {
      key: "a2a",
      label: "A2A Client",
      children: <A2AClientTab />,
    },
  ];

  return (
    <Container>
      <h1>Playground</h1>
      <p style={{ marginBottom: "1.5rem", color: "#666" }}>
        Test your configured routes and backends
      </p>

      <RouteSelection />

      <Card>
        <Tabs defaultActiveKey="http" items={items} />
      </Card>

      {response && (
        <Card style={{ marginTop: "1rem" }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "0.5rem",
              marginBottom: "1rem",
            }}
          >
            <span style={{ fontSize: "18px", fontWeight: 500 }}>Response</span>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: "1rem",
                marginLeft: "auto",
              }}
            >
              <StatusBadge success={response.status < 400}>
                {response.status} {response.statusText}
              </StatusBadge>
              <span
                style={{
                  color: "var(--color-text-secondary)",
                  fontSize: "14px",
                }}
              >
                Duration: {response.responseTime.toFixed(2)}ms
              </span>
            </div>
          </div>

          <Tabs defaultActiveKey="body" type="card" size="small">
            <Tabs.TabPane tab="Response Body" key="body">
              <CodeBlock>{response.body}</CodeBlock>
            </Tabs.TabPane>
            <Tabs.TabPane tab="Headers" key="headers">
              <CodeBlock>{JSON.stringify(response.headers, null, 2)}</CodeBlock>
            </Tabs.TabPane>
          </Tabs>
        </Card>
      )}
    </Container>
  );
};
