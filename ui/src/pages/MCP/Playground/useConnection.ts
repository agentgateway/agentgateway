import { Client as McpClient } from "@modelcontextprotocol/sdk/client/index.js";
import { SSEClientTransport as McpSseTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import {
  McpError,
  ListToolsResultSchema as McpListToolsResultSchema,
  type ClientRequest as McpClientRequest,
  type Tool as McpTool,
} from "@modelcontextprotocol/sdk/types.js";
import { useState } from "react";
import toast from "react-hot-toast";
import { z } from "zod";
import type { ConnectionState, McpState, RouteInfo, UiState } from "./types";

const McpToolResponseSchema = z.any();

export function useConnection(
  selectedRoute: RouteInfo | null,
  _routes: RouteInfo[],
) {
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

  const resetConnectionForRoute = () => {
    setConnectionState((prev) => ({ ...prev, isConnected: false }));
    setMcpState({
      client: null,
      tools: [],
      selectedTool: null,
      paramValues: {},
      response: null,
    });
  };

  const handleAuthTokenChange = (token: string) => {
    setConnectionState((prev) => ({ ...prev, authToken: token }));
  };

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

  return {
    connectionState,
    mcpState,
    uiState,
    resetConnectionForRoute,
    handleAuthTokenChange,
    connect,
    runMcpTool,
    handleMcpToolSelect,
    handleMcpParamChange,
  };
}
