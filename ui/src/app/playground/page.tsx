"use client";

import { useState, useEffect, useCallback } from "react";
import { Client as McpClient } from "@modelcontextprotocol/sdk/client/index.js";
import { SSEClientTransport as McpSseTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import {
  ClientRequest as McpClientRequest,
  Result as McpResult,
  Request as McpRequest,
  McpError,
  ListToolsResultSchema as McpListToolsResultSchema,
  Tool as McpTool,
} from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import {
  A2AClient,
  createAuthenticatingFetchWithRetry,
  type AuthenticationHandler,
} from "@a2a-js/sdk/client";
import type { AgentSkill, Task, Message, MessageSendParams, AgentCard } from "@a2a-js/sdk";
import { useServer } from "@/lib/server-context";
import { Bind, Listener, Route, Backend, ListenerProtocol } from "@/lib/types";
import { toast } from "sonner";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Globe,
  ArrowRight,
  Server,
  Send,
  Loader2,
  Clock,
  Shield,
  Settings,
  CheckCircle,
  AlertCircle,
  Info,
  Users,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { getBackendName } from "@/lib/backend-utils";
import { v4 as uuidv4 } from "uuid";

import { CapabilitiesList } from "@/components/playground/CapabilitiesList";
import { ActionPanel } from "@/components/playground/ActionPanel";
import { ResponseDisplay } from "@/components/playground/ResponseDisplay";

// Schema for MCP tool invocation response
const McpToolResponseSchema = z.any();

// Interface for route testing
interface RouteInfo {
  bindPort: number;
  listener: Listener;
  route: Route;
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
  selectedListenerProtocol: ListenerProtocol | null;
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

const HTTP_METHODS = ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];

export default function PlaygroundPage() {
  const { binds } = useServer();
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

  // Determine backend type of selected route
  const getRouteBackendType = (route: RouteInfo): "mcp" | "a2a" | "http" => {
    // Check if route has A2A policy first - this takes precedence
    if (route.route.policies?.a2a) {
      return "a2a";
    }

    if (!route.route.backends || route.route.backends.length === 0) return "http";

    const backend = route.route.backends[0]; // Use first backend to determine type
    if (backend.mcp) return "mcp";
    if (backend.ai) return "a2a"; // Treat AI backends as A2A for now
    return "http"; // Host, Service, etc.
  };

  const updateRequestFromRoute = useCallback((routeInfo: RouteInfo) => {
    let initialPath = "/";

    if (routeInfo.route.matches && routeInfo.route.matches.length > 0) {
      const firstMatch = routeInfo.route.matches[0];
      if (firstMatch.path.exact) {
        initialPath = firstMatch.path.exact;
      } else if (firstMatch.path.pathPrefix) {
        initialPath = firstMatch.path.pathPrefix;
      } else if (firstMatch.path.regex) {
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
      selectedListenerProtocol: routeInfo.listener.protocol,
    }));
  }, []);

  // Extract routes from configuration
  useEffect(() => {
    if (!binds || binds.length === 0) return;

    const extractedRoutes: RouteInfo[] = [];

    binds.forEach((bind: Bind) => {
      bind.listeners.forEach((listener: Listener) => {
        if (listener.routes) {
          listener.routes.forEach((route: Route, routeIndex: number) => {
            const protocol = listener.protocol === ListenerProtocol.HTTPS ? "https" : "http";
            const hostname = listener.hostname || "localhost";
            const port = bind.port; // Use the actual port from the bind configuration
            const baseEndpoint = `${protocol}://${hostname}:${port}`;

            // Generate route path and description with better pattern recognition
            let routePath = "/";
            let routePattern = "/*";

            if (route.matches?.[0]?.path) {
              const pathMatch = route.matches[0].path;
              if (pathMatch.exact) {
                routePath = pathMatch.exact;
                routePattern = pathMatch.exact;
              } else if (pathMatch.pathPrefix) {
                routePath = pathMatch.pathPrefix;
                routePattern = pathMatch.pathPrefix + "*";
              } else if (pathMatch.regex) {
                routePath = "/";
                routePattern = `~${pathMatch.regex}`;
              }
            }

            // Create full endpoint with route path
            const endpoint = `${baseEndpoint}${routePath}`;

            const hostnames = route.hostnames?.join(", ") || "";
            const backendCount = route.backends?.length || 0;
            const hasA2aPolicy = route.policies?.a2a;
            const backendTypes =
              route.backends
                ?.map((b) => {
                  if (b.mcp) return "MCP";
                  if (b.ai) return "AI";
                  if (b.service) return "Service";
                  if (b.host) return "Host";
                  if (b.dynamic) return "Dynamic";
                  return "Unknown";
                })
                .join(", ") || "";

            const policyInfo = hasA2aPolicy ? "A2A Policy" : "";
            const routeDescription = `${routePattern}${hostnames ? ` • ${hostnames}` : ""} • ${backendCount} backend${backendCount !== 1 ? "s" : ""}${backendTypes ? ` (${backendTypes})` : ""}${policyInfo}`;

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
      });
    });

    setRoutes(extractedRoutes);

    // Auto-select first route if available
    if (extractedRoutes.length > 0 && !selectedRoute) {
      setSelectedRoute(extractedRoutes[0]);
      updateRequestFromRoute(extractedRoutes[0]);
    }
  }, [binds, selectedRoute, updateRequestFromRoute]);

  // (moved getRouteBackendType and updateRequestFromRoute above useEffect)

  const handleRouteSelect = (routeInfo: RouteInfo) => {
    // Don't allow selection of routes with no backends unless they have A2A policy
    const hasBackends = routeInfo.route.backends && routeInfo.route.backends.length > 0;
    const hasA2aPolicy = routeInfo.route.policies?.a2a;

    if (!hasBackends && !hasA2aPolicy) {
      toast.error(
        "Cannot test route without backends or A2A policy. Please configure at least one backend or enable A2A policy for this route."
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
      headers: Object.fromEntries(Object.entries(prev.headers).filter(([k]) => k !== key)),
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
      query: Object.fromEntries(Object.entries(prev.query).filter(([k]) => k !== key)),
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

      const response = await fetch(url.toString(), fetchOptions);
      const endTime = performance.now();
      const responseTime = endTime - startTime;

      const responseBody = await response.text();
      const responseHeaders: Record<string, string> = {};
      response.headers.forEach((value, key) => {
        responseHeaders[key] = value;
      });

      setResponse({
        status: response.status,
        statusText: response.statusText,
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
          { capabilities: {} }
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
        const listToolsRequest: McpClientRequest = { method: "tools/list", params: {} };
        const toolsResponse = await client.request(listToolsRequest, McpListToolsResultSchema);
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
            shouldRetryWithHeaders: async (req: RequestInit, res: Response) => {
              // Retry with auth headers on 401 or 403 responses
              if (res.status === 401 || res.status === 403) {
                return {
                  Authorization: `Bearer ${connectionState.authToken}`,
                };
              }
              return undefined;
            },
          };

          const authenticatedFetch = createAuthenticatingFetchWithRetry(fetch, authHandler);
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
            `Loaded A2A agent: ${agentCard.name} with ${skills.length} skill${skills.length !== 1 ? "s" : ""}`
          );
        } catch (error: any) {
          console.error("Failed to load A2A capabilities:", error);
          // Don't fail the connection, just continue without skills
          setA2aState((prev) => ({ ...prev, skills: [] }));

          // Provide specific guidance for CORS errors
          let errorMessage = "Unknown error loading agent card";
          if (error instanceof Error) {
            if (
              error.message.includes("CORS") ||
              error.message.includes("Access-Control-Allow-Origin")
            ) {
              errorMessage = "CORS error - A2A endpoint needs to allow cross-origin requests";
            } else if (
              error.message.includes("NetworkError") ||
              error.message.includes("Failed to fetch")
            ) {
              errorMessage = "Network error - check if A2A endpoint is reachable and allows CORS";
            } else {
              errorMessage = error.message;
            }
          }

          toast.warning(`Connected to A2A endpoint but couldn't load agent card: ${errorMessage}`);
        }
      }
    } catch (error: any) {
      console.error("Failed to connect:", error);
      console.error("Error details:", {
        name: error?.name,
        message: error?.message,
        code: error?.code,
        stack: error?.stack,
        cause: error?.cause,
      });

      const errorMessage =
        error instanceof McpError || error instanceof Error
          ? error.message
          : "Unknown connection error";

      // Enhanced error detection and messaging
      if (errorMessage.includes("401") || error?.code === 401) {
        toast.error("❌ Unauthorized (401): Check bearer token or remove it if not needed");
      } else if (errorMessage.includes("406") || error?.code === 406) {
        toast.error(
          "❌ Not Acceptable (406): Server rejected request headers - check CORS configuration"
        );
      } else if (
        errorMessage.includes("Not Found") ||
        errorMessage.includes("404") ||
        error?.code === 404
      ) {
        toast.error(
          `❌ Not Found (404): Endpoint '${selectedRoute?.endpoint || "unknown"}' not found`
        );
      } else if (
        errorMessage.includes("Failed to fetch") ||
        errorMessage.includes("NetworkError") ||
        errorMessage.includes("ERR_NETWORK") ||
        error?.name === "TypeError"
      ) {
        // Enhanced CORS/Network error detection
        let detailedMessage = "❌ Connection Failed - Possible causes:\n";
        detailedMessage += "• CORS: Server needs 'Access-Control-Allow-Origin' header\n";
        detailedMessage += "• Network: Server may be down or unreachable\n";
        detailedMessage += "• Headers: Missing required headers (Accept, mcp-protocol-version)\n";
        detailedMessage += `• URL: Check if '${selectedRoute?.endpoint || "unknown"}/sse' is correct\n`;
        detailedMessage += "• Config: Verify agentgateway is running with correct config";

        console.error("CORS/Network error details:", {
          url: `${selectedRoute?.endpoint || "unknown"}/sse`,
          headers: error?.headers,
          mode: "cors",
          credentials: "omit",
        });

        toast.error(detailedMessage, { duration: 8000 });
      } else if (errorMessage.includes("CORS") || errorMessage.includes("Access-Control")) {
        toast.error(
          `❌ CORS Error: ${errorMessage}\n• Add '${window.location.origin}' to CORS allowOrigins\n• Check CORS headers in agentgateway config`,
          { duration: 6000 }
        );
      } else {
        toast.error(`❌ Connection failed: ${errorMessage}`);
      }
      resetFullStateAfterDisconnect();
    } finally {
      setConnectionState((prev) => ({ ...prev, isConnecting: false }));
      setUiState((prev) => ({ ...prev, isLoadingCapabilities: false }));
    }
  };

  const disconnect = async () => {
    if (connectionState.connectionType === "mcp" && mcpState.client) {
      try {
        await mcpState.client.close();
      } catch (e) {
        console.error("Error closing MCP client:", e);
      }
    }

    if (connectionState.connectionType === "a2a" && a2aState.client) {
      try {
        if ("close" in a2aState.client && typeof (a2aState.client as any).close === "function") {
          await (a2aState.client as any).close();
        } else if (
          "disconnect" in a2aState.client &&
          typeof (a2aState.client as any).disconnect === "function"
        ) {
          await (a2aState.client as any).disconnect();
        }
      } catch (e) {
        console.error("Error closing A2A client:", e);
      }
    }

    resetFullStateAfterDisconnect();
    toast.info("Disconnected");
  };

  const resetClientState = () => {
    setConnectionState((prev) => ({ ...prev, connectionType: connectionState.connectionType }));
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

  const resetFullStateAfterDisconnect = () => {
    setConnectionState((prev) => ({
      ...prev,
      isConnected: false,
      isConnecting: false,
      isLoadingA2aTargets: false,
    }));
    resetClientState();
  };

  const handleMcpToolSelect = (tool: McpTool) => {
    setMcpState((prev) => ({ ...prev, selectedTool: tool, response: null }));
    setA2aState((prev) => ({ ...prev, response: null }));

    const initialParams: Record<string, any> = {};
    Object.entries(tool.inputSchema.properties || {}).forEach(([key, prop]: [string, any]) => {
      switch (prop.type) {
        case "boolean":
          initialParams[key] = false;
          break;
        case "number":
        case "integer":
          initialParams[key] = 0;
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
    setMcpState((prev) => ({ ...prev, paramValues: initialParams }));
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
      const result = await mcpState.client.request(request, McpToolResponseSchema);
      setMcpState((prev) => ({ ...prev, response: result }));
      toast.success(`Tool ${mcpState.selectedTool?.name} executed.`);
    } catch (error: any) {
      const message = error instanceof McpError ? error.message : "Failed to run tool";
      setMcpState((prev) => ({ ...prev, response: { error: message, details: error } }));
      toast.error(message);
    } finally {
      setUiState((prev) => ({ ...prev, isRequestRunning: false }));
    }
  };

  const handleA2aSkillSelect = (skill: AgentSkill) => {
    setA2aState((prev) => ({ ...prev, selectedSkill: skill, response: null, message: "" }));
    setMcpState((prev) => ({ ...prev, response: null }));
  };

  const runA2aSkill = async () => {
    if (!a2aState.client || !a2aState.selectedSkill || !a2aState.message.trim()) {
      if (!a2aState.message.trim()) toast.warning("Please enter a message for the agent.");
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
      toast.success(`Task sent to agent using skill ${a2aState.selectedSkill?.name}.`);
    } catch (error: any) {
      console.error("Failed to run A2A skill:", error);
      const message = error instanceof Error ? `Error: ${error.message}` : "Failed to send task";
      setA2aState((prev) => ({ ...prev, response: { error: message, details: error } }));
      toast.error(message);
    } finally {
      setUiState((prev) => ({ ...prev, isRequestRunning: false }));
    }
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

  const getBackendInfo = (backend: Backend) => {
    if (backend.mcp) {
      return { type: "MCP", name: getBackendName(backend), icon: Server };
    } else if (backend.host) {
      return {
        type: "Host",
        name: backend.host.Hostname?.[0] || backend.host.Address || "Unknown",
        icon: Globe,
      };
    } else if (backend.service) {
      return { type: "Service", name: backend.service.name.hostname, icon: Server };
    } else if (backend.ai) {
      return { type: "AI", name: backend.ai.name, icon: Settings };
    }
    return { type: "Unknown", name: "Unknown", icon: AlertCircle };
  };

  const getStatusColor = (status: number) => {
    if (status >= 200 && status < 300) return "text-green-600";
    if (status >= 300 && status < 400) return "text-primary";
    if (status >= 400 && status < 500) return "text-orange-600";
    if (status >= 500) return "text-red-600";
    return "text-gray-600";
  };

  const getStatusIcon = (status: number) => {
    if (status >= 200 && status < 300) return CheckCircle;
    if (status >= 300 && status < 400) return Info;
    return AlertCircle;
  };

  return (
    <div className="container mx-auto py-8 px-4 space-y-6">
      <div className="mb-6">
        <h1 className="text-3xl font-bold tracking-tight">Playground</h1>
        <p className="text-muted-foreground mt-1">Test your configured routes and backends</p>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        {/* Routes Panel */}
        <Card className="lg:col-span-1">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Globe className="h-5 w-5" />
              Routes
            </CardTitle>
            <CardDescription>
              {routes.length === 0
                ? "No routes configured"
                : `${routes.length} route${routes.length !== 1 ? "s" : ""} available`}
            </CardDescription>
          </CardHeader>
          <CardContent>
            {routes.length === 0 ? (
              <div className="text-center py-8 text-muted-foreground">
                <Globe className="h-12 w-12 mx-auto mb-4 opacity-50" />
                <p>No routes configured</p>
                <p className="text-sm mt-2">Add routes in the configuration to test them here</p>
              </div>
            ) : (
              <div className="space-y-3">
                {(() => {
                  // Group routes by bind port and listener
                  const groupedRoutes = new Map<string, RouteInfo[]>();
                  routes.forEach((routeInfo) => {
                    const groupKey = `${routeInfo.bindPort}-${routeInfo.listener.name || "unnamed"}`;
                    if (!groupedRoutes.has(groupKey)) {
                      groupedRoutes.set(groupKey, []);
                    }
                    groupedRoutes.get(groupKey)!.push(routeInfo);
                  });

                  return Array.from(groupedRoutes.entries()).map(([groupKey, routeInfos]) => {
                    const firstRoute = routeInfos[0];
                    const port = firstRoute.bindPort;
                    const listenerName = firstRoute.listener.name || "unnamed";
                    const endpoint = firstRoute.endpoint;

                    return (
                      <div key={groupKey} className="border rounded-lg bg-background">
                        {/* Group Header */}
                        <div className="p-3 border-b bg-muted/30">
                          <div className="flex items-center justify-between">
                            <div className="flex items-center gap-2">
                              <Server className="h-4 w-4 text-muted-foreground" />
                              <span className="font-medium text-sm">{listenerName}</span>
                              <Badge className="text-xs">Port {port}</Badge>
                            </div>
                            <div className="text-xs text-muted-foreground font-mono">
                              {endpoint}
                            </div>
                          </div>
                        </div>

                        {/* Routes in this group */}
                        <div className="divide-y">
                          {routeInfos.map((routeInfo, index) => {
                            const hasBackends =
                              routeInfo.route.backends && routeInfo.route.backends.length > 0;
                            const hasA2aPolicy = routeInfo.route.policies?.a2a;
                            const backendTypes =
                              routeInfo.route.backends?.map((b) => {
                                if (b.mcp) return "MCP";
                                if (b.ai) return "AI";
                                if (b.service) return "Service";
                                if (b.host) return "Host";
                                if (b.dynamic) return "Dynamic";
                                return "Unknown";
                              }) || [];

                            return (
                              <div
                                key={`${groupKey}-${index}`}
                                className={cn(
                                  "p-3 transition-colors",
                                  !hasBackends && !hasA2aPolicy
                                    ? "bg-destructive/5 cursor-not-allowed opacity-75"
                                    : selectedRoute === routeInfo
                                      ? "bg-primary/10 cursor-pointer"
                                      : "hover:bg-muted/50 cursor-pointer"
                                )}
                                onClick={() => handleRouteSelect(routeInfo)}
                              >
                                <div className="flex items-center justify-between">
                                  <div className="flex-1 min-w-0">
                                    {/* Route name and path */}
                                    <div className="flex items-center gap-2 mb-2">
                                      {!hasBackends && !hasA2aPolicy && (
                                        <AlertCircle className="h-4 w-4 text-destructive flex-shrink-0" />
                                      )}
                                      <span className="font-medium">
                                        {routeInfo.route.name ||
                                          `Route ${routeInfo.routeIndex + 1}`}
                                      </span>
                                      <Badge variant="outline" className="text-xs font-mono">
                                        {routeInfo.routePath}
                                      </Badge>
                                      {routeInfo.route.matches?.[0]?.path?.regex && (
                                        <Badge className="text-xs">regex</Badge>
                                      )}
                                      {routeInfo.route.matches?.[0]?.path?.pathPrefix && (
                                        <Badge className="text-xs">prefix</Badge>
                                      )}
                                      {routeInfo.route.matches?.[0]?.path?.exact && (
                                        <Badge className="text-xs">exact</Badge>
                                      )}
                                    </div>

                                    {/* Route details */}
                                    <div className="text-sm text-muted-foreground space-y-1">
                                      {/* Hostnames */}
                                      {routeInfo.route.hostnames &&
                                        routeInfo.route.hostnames.length > 0 && (
                                          <div className="flex items-center gap-1 text-xs">
                                            <Globe className="h-3 w-3" />
                                            <span>
                                              Hosts: {routeInfo.route.hostnames.join(", ")}
                                            </span>
                                          </div>
                                        )}

                                      {/* Backends */}
                                      <div className="flex items-center gap-2 text-xs">
                                        <div
                                          className={cn(
                                            "flex items-center gap-1",
                                            !hasBackends && !hasA2aPolicy && "text-destructive"
                                          )}
                                        >
                                          <Server className="h-3 w-3" />
                                          <span>
                                            {hasA2aPolicy && !hasBackends
                                              ? "A2A Traffic"
                                              : `${routeInfo.route.backends?.length || 0} backend${
                                                  (routeInfo.route.backends?.length || 0) !== 1
                                                    ? "s"
                                                    : ""
                                                }`}
                                          </span>
                                        </div>

                                        {/* Backend types and A2A policy */}
                                        {(hasBackends || hasA2aPolicy) && (
                                          <div className="flex gap-1">
                                            {hasA2aPolicy && (
                                              <Badge
                                                variant="default"
                                                className="text-xs py-0 px-1 bg-primary hover:bg-primary/90"
                                              >
                                                A2A
                                              </Badge>
                                            )}
                                            {hasBackends &&
                                              backendTypes.map((type, idx) => (
                                                <Badge key={idx} className="text-xs py-0 px-1">
                                                  {type}
                                                </Badge>
                                              ))}
                                          </div>
                                        )}
                                      </div>

                                      {/* Error message */}
                                      {!hasBackends && !hasA2aPolicy && (
                                        <div className="text-destructive text-xs mt-1 flex items-center gap-1">
                                          <AlertCircle className="h-3 w-3" />
                                          <span>Cannot test - no backends configured</span>
                                        </div>
                                      )}
                                    </div>
                                  </div>

                                  {(hasBackends || hasA2aPolicy) && (
                                    <ArrowRight className="h-4 w-4 text-muted-foreground flex-shrink-0" />
                                  )}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    );
                  });
                })()}
              </div>
            )}
          </CardContent>
        </Card>

        {/* Testing Panel */}
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Send className="h-5 w-5" />
              Testing
            </CardTitle>
            <CardDescription>
              {selectedRoute
                ? `Test ${getRouteBackendType(selectedRoute)} backend`
                : "Select a route to start testing"}
            </CardDescription>
          </CardHeader>
          <CardContent>
            {!selectedRoute ? (
              <div className="text-center py-8 text-muted-foreground">
                <Send className="h-12 w-12 mx-auto mb-4 opacity-50" />
                <p>Select a route to start testing</p>
              </div>
            ) : connectionState.connectionType === "http" ? (
              // HTTP Testing Interface
              <div className="space-y-4">
                {/* Request URL Display */}
                <div className="bg-muted/30 rounded-lg p-4 border">
                  <div className="flex items-center gap-2 mb-2">
                    <Globe className="h-4 w-4 text-muted-foreground" />
                    <span className="font-medium text-sm">Request URL</span>
                  </div>
                  <div className="font-mono text-sm break-all">
                    {selectedRoute.protocol}://{selectedRoute.listener.hostname || "localhost"}:
                    {selectedRoute.bindPort}
                    {request.path}
                  </div>
                </div>

                {/* Method and Path Configuration */}
                <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
                  <div className="space-y-2">
                    <Label>HTTP Method</Label>
                    <Select
                      value={request.method}
                      onValueChange={(value) => setRequest((prev) => ({ ...prev, method: value }))}
                    >
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        {HTTP_METHODS.map((method) => (
                          <SelectItem key={method} value={method}>
                            {method}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>

                  <div className="md:col-span-3 space-y-2">
                    <Label>
                      Request Path
                      {selectedRoute.route.matches?.[0]?.path?.regex && (
                        <span className="text-xs text-muted-foreground ml-2">
                          (Must match pattern: {selectedRoute.route.matches[0].path.regex})
                        </span>
                      )}
                      {selectedRoute.route.matches?.[0]?.path?.pathPrefix && (
                        <span className="text-xs text-muted-foreground ml-2">
                          (Must start with: {selectedRoute.route.matches[0].path.pathPrefix})
                        </span>
                      )}
                      {selectedRoute.route.matches?.[0]?.path?.exact && (
                        <span className="text-xs text-muted-foreground ml-2">
                          (Must be exactly: {selectedRoute.route.matches[0].path.exact})
                        </span>
                      )}
                    </Label>
                    <div className="flex gap-2">
                      <Input
                        value={request.path}
                        onChange={(e) => setRequest((prev) => ({ ...prev, path: e.target.value }))}
                        placeholder={
                          selectedRoute.route.matches?.[0]?.path?.regex
                            ? "/your/path/here"
                            : selectedRoute.route.matches?.[0]?.path?.pathPrefix
                              ? `${selectedRoute.route.matches[0].path.pathPrefix}...`
                              : selectedRoute.route.matches?.[0]?.path?.exact
                                ? selectedRoute.route.matches[0].path.exact
                                : "/path"
                        }
                        className="flex-1"
                      />
                      <Button onClick={sendHttpRequest} disabled={isLoading}>
                        {isLoading ? (
                          <>
                            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                            Sending...
                          </>
                        ) : (
                          <>
                            <Send className="mr-2 h-4 w-4" />
                            Send
                          </>
                        )}
                      </Button>
                    </div>
                  </div>
                </div>

                {/* Route Info */}
                <div className="bg-muted/30 rounded-lg p-4 border">
                  <h4 className="font-medium mb-2 flex items-center gap-2">
                    <Settings className="h-4 w-4" />
                    Route Configuration
                  </h4>
                  <div className="space-y-2 text-sm">
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Name:</span>
                      <span>{selectedRoute.route.name || "Unnamed"}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Listener:</span>
                      <span>{selectedRoute.listener.name || "Unnamed"}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Port:</span>
                      <span>{selectedRoute.bindPort}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-muted-foreground">Route Pattern:</span>
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-xs">{selectedRoute.routePath}</span>
                        {selectedRoute.route.matches?.[0]?.path?.regex && (
                          <Badge className="text-xs">regex</Badge>
                        )}
                        {selectedRoute.route.matches?.[0]?.path?.pathPrefix && (
                          <Badge className="text-xs">prefix</Badge>
                        )}
                        {selectedRoute.route.matches?.[0]?.path?.exact && (
                          <Badge className="text-xs">exact</Badge>
                        )}
                      </div>
                    </div>
                    {selectedRoute.route.hostnames && selectedRoute.route.hostnames.length > 0 && (
                      <div className="flex justify-between">
                        <span className="text-muted-foreground">Host Match:</span>
                        <span className="text-xs">{selectedRoute.route.hostnames.join(", ")}</span>
                      </div>
                    )}
                    <div>
                      <span className="text-muted-foreground">Backends:</span>
                      <div className="mt-1 space-y-1">
                        {selectedRoute.route.backends?.map((backend, idx) => {
                          const info = getBackendInfo(backend);
                          const Icon = info.icon;
                          return (
                            <div key={idx} className="flex items-center gap-2">
                              <Icon className="h-3 w-3" />
                              <Badge className="text-xs">{info.type}</Badge>
                              <span className="text-xs">{info.name}</span>
                              {backend.weight && backend.weight !== 1 && (
                                <span className="text-xs text-muted-foreground">
                                  (weight: {backend.weight})
                                </span>
                              )}
                            </div>
                          );
                        }) || (
                          <div className="text-xs text-muted-foreground">
                            No backends configured
                          </div>
                        )}
                      </div>
                    </div>
                    {selectedRoute.route.policies &&
                      Object.keys(selectedRoute.route.policies).length > 0 && (
                        <div>
                          <span className="text-muted-foreground">Policies:</span>
                          <div className="mt-1 flex flex-wrap gap-1">
                            {Object.entries(selectedRoute.route.policies).map(
                              ([policyType, policyConfig]) => {
                                // Skip null/undefined policies
                                if (!policyConfig) return null;

                                // Get policy display info
                                const getPolicyInfo = (type: string) => {
                                  switch (type) {
                                    case "jwtAuth":
                                      return { name: "JWT Auth", icon: Shield };
                                    case "mcpAuthentication":
                                      return { name: "MCP Auth", icon: Shield };
                                    case "mcpAuthorization":
                                      return { name: "MCP Authz", icon: Shield };
                                    case "cors":
                                      return { name: "CORS", icon: Globe };
                                    case "backendTLS":
                                      return { name: "Backend TLS", icon: Shield };
                                    case "backendAuth":
                                      return { name: "Backend Auth", icon: Shield };
                                    case "localRateLimit":
                                      return { name: "Local Rate Limit", icon: Clock };
                                    case "remoteRateLimit":
                                      return { name: "Remote Rate Limit", icon: Clock };
                                    case "timeout":
                                      return { name: "Timeout", icon: Clock };
                                    case "retry":
                                      return { name: "Retry", icon: ArrowRight };
                                    case "requestHeaderModifier":
                                      return { name: "Request Headers", icon: Settings };
                                    case "responseHeaderModifier":
                                      return { name: "Response Headers", icon: Settings };
                                    case "requestRedirect":
                                      return { name: "Redirect", icon: ArrowRight };
                                    case "urlRewrite":
                                      return { name: "URL Rewrite", icon: Settings };
                                    case "directResponse":
                                      return { name: "Direct Response", icon: Server };
                                    case "extAuthz":
                                      return { name: "External Auth", icon: Shield };
                                    case "ai":
                                      return { name: "AI Policy", icon: Settings };
                                    case "a2a":
                                      return { name: "A2A", icon: Users };
                                    default:
                                      return { name: type, icon: Settings };
                                  }
                                };

                                const info = getPolicyInfo(policyType);
                                const Icon = info.icon;

                                return (
                                  <Badge key={policyType} variant="outline" className="text-xs">
                                    <Icon className="h-3 w-3 mr-1" />
                                    {info.name}
                                  </Badge>
                                );
                              }
                            )}
                          </div>
                        </div>
                      )}
                  </div>
                </div>

                <Tabs defaultValue="headers" className="w-full">
                  <TabsList className="grid w-full grid-cols-3">
                    <TabsTrigger value="headers">Headers</TabsTrigger>
                    <TabsTrigger value="query">Query</TabsTrigger>
                    <TabsTrigger value="body">Body</TabsTrigger>
                  </TabsList>

                  <TabsContent value="headers" className="space-y-4">
                    <div className="flex gap-2">
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
                      <Button onClick={addHeader} variant="outline">
                        Add
                      </Button>
                    </div>
                    <div className="space-y-2">
                      {Object.entries(request.headers).map(([key, value]) => (
                        <div
                          key={key}
                          className="flex items-center justify-between p-2 bg-muted/30 rounded"
                        >
                          <span className="text-sm">
                            <span className="font-medium">{key}:</span> {value}
                          </span>
                          <Button variant="ghost" size="sm" onClick={() => removeHeader(key)}>
                            Remove
                          </Button>
                        </div>
                      ))}
                    </div>
                  </TabsContent>

                  <TabsContent value="query" className="space-y-4">
                    <div className="flex gap-2">
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
                      <Button onClick={addQuery} variant="outline">
                        Add
                      </Button>
                    </div>
                    <div className="space-y-2">
                      {Object.entries(request.query).map(([key, value]) => (
                        <div
                          key={key}
                          className="flex items-center justify-between p-2 bg-muted/30 rounded"
                        >
                          <span className="text-sm">
                            <span className="font-medium">{key}:</span> {value}
                          </span>
                          <Button variant="ghost" size="sm" onClick={() => removeQuery(key)}>
                            Remove
                          </Button>
                        </div>
                      ))}
                    </div>
                  </TabsContent>

                  <TabsContent value="body" className="space-y-4">
                    <div>
                      <Label htmlFor="request-body">Request Body</Label>
                      <Textarea
                        id="request-body"
                        placeholder="Enter request body (JSON, XML, etc.)"
                        value={request.body}
                        onChange={(e) => setRequest((prev) => ({ ...prev, body: e.target.value }))}
                        className="mt-2 min-h-32"
                      />
                    </div>
                  </TabsContent>
                </Tabs>
              </div>
            ) : (
              // MCP/A2A Testing Interface
              <div className="space-y-4">
                {/* Connection Status */}
                <div className="bg-muted/30 rounded-lg p-4 border">
                  <div className="flex items-center gap-2 mb-2">
                    <Server className="h-4 w-4 text-muted-foreground" />
                    <span className="font-medium text-sm">Connection</span>
                  </div>
                  <div className="space-y-3">
                    <div className="flex justify-between items-center">
                      <span className="text-sm text-muted-foreground">Endpoint:</span>
                      <span className="font-mono text-sm">{connectionState.selectedEndpoint}</span>
                    </div>
                    <div className="flex justify-between items-center">
                      <span className="text-sm text-muted-foreground">Type:</span>
                      <Badge className="text-xs">
                        {connectionState.connectionType?.toUpperCase()}
                      </Badge>
                    </div>
                    <div className="flex items-center gap-2">
                      <Label htmlFor="auth-token" className="text-sm">
                        Bearer Token (Optional):
                      </Label>
                      <Input
                        id="auth-token"
                        placeholder="Enter token if required"
                        type="password"
                        value={connectionState.authToken}
                        onChange={(e) => handleAuthTokenChange(e.target.value)}
                        disabled={connectionState.isConnected || connectionState.isConnecting}
                        className="flex-1"
                      />
                      <Button
                        onClick={connectionState.isConnected ? disconnect : connect}
                        disabled={connectionState.isConnecting}
                        className="w-[130px]"
                      >
                        {connectionState.isConnecting ? (
                          <>
                            <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                            Connecting...
                          </>
                        ) : connectionState.isConnected ? (
                          "Disconnect"
                        ) : (
                          "Connect"
                        )}
                      </Button>
                    </div>
                  </div>
                </div>
              </div>
            )}
          </CardContent>
        </Card>
      </div>

      {connectionState.connectionType !== "http" && connectionState.isConnected && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          <CapabilitiesList
            mcpTools={mcpState.tools}
            a2aSkills={a2aState.skills}
            a2aAgentCard={a2aState.agentCard}
            connectionType={connectionState.connectionType}
            isLoading={uiState.isLoadingCapabilities}
            selectedMcpToolName={mcpState.selectedTool?.name ?? null}
            selectedA2aSkillId={a2aState.selectedSkill?.id ?? null}
            onMcpToolSelect={handleMcpToolSelect}
            onA2aSkillSelect={handleA2aSkillSelect}
          />

          <ActionPanel
            connectionType={connectionState.connectionType}
            mcpSelectedTool={mcpState.selectedTool}
            a2aSelectedSkill={a2aState.selectedSkill}
            mcpParamValues={mcpState.paramValues}
            a2aMessage={a2aState.message}
            isRequestRunning={uiState.isRequestRunning}
            onMcpParamChange={handleMcpParamChange}
            onA2aMessageChange={handleA2aMessageChange}
            onRunMcpTool={runMcpTool}
            onRunA2aSkill={runA2aSkill}
          />
        </div>
      )}

      {/* Response Panel */}
      {(response || mcpState.response || a2aState.response) && (
        <>
          {/* HTTP Response */}
          {response && connectionState.connectionType === "http" && (
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <div className="flex items-center gap-2">
                    {(() => {
                      const StatusIcon = getStatusIcon(response.status);
                      return <StatusIcon className="h-5 w-5" />;
                    })()}
                    Response
                  </div>
                  <div className="ml-auto flex items-center gap-4 text-sm text-muted-foreground">
                    <div className="flex items-center gap-1">
                      <Clock className="h-4 w-4" />
                      {response.responseTime.toFixed(2)}ms
                    </div>
                    <div className={cn("font-medium", getStatusColor(response.status))}>
                      {response.status} {response.statusText}
                    </div>
                  </div>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <Tabs defaultValue="body" className="w-full">
                  <TabsList className="grid w-full grid-cols-2">
                    <TabsTrigger value="body">Response Body</TabsTrigger>
                    <TabsTrigger value="headers">Headers</TabsTrigger>
                  </TabsList>

                  <TabsContent value="body" className="space-y-4">
                    <div>
                      <Label>Response Body</Label>
                      <Textarea
                        value={response.body}
                        readOnly
                        className="mt-2 min-h-64 font-mono text-sm"
                      />
                    </div>
                  </TabsContent>

                  <TabsContent value="headers" className="space-y-4">
                    <div className="space-y-2">
                      {Object.entries(response.headers).map(([key, value]) => (
                        <div
                          key={key}
                          className="flex items-center justify-between p-2 bg-muted/30 rounded"
                        >
                          <span className="text-sm">
                            <span className="font-medium">{key}:</span> {value}
                          </span>
                        </div>
                      ))}
                    </div>
                  </TabsContent>
                </Tabs>
              </CardContent>
            </Card>
          )}

          {/* MCP/A2A Response */}
          {(mcpState.response || a2aState.response) &&
            connectionState.connectionType !== "http" && (
              <ResponseDisplay
                mcpResponse={mcpState.response}
                a2aResponse={a2aState.response}
                connectionType={connectionState.connectionType}
              />
            )}
        </>
      )}
    </div>
  );
}
