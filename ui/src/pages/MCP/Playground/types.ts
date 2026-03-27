import { Client as McpClient } from "@modelcontextprotocol/sdk/client/index.js";
import type {
  Request as McpRequest,
  Result as McpResult,
  Tool as McpTool,
} from "@modelcontextprotocol/sdk/types.js";
import type { LocalListener, LocalRoute } from "../../../config";

export interface RouteInfo {
  bindPort: number;
  listener: LocalListener;
  route: LocalRoute;
  endpoint: string;
  protocol: string;
  routeIndex: number;
  routePath: string;
}

export interface ConnectionState {
  authToken: string;
  isConnected: boolean;
  isConnecting: boolean;
}

export interface McpState {
  client: McpClient<McpRequest, any, McpResult> | null;
  tools: McpTool[];
  selectedTool: McpTool | null;
  paramValues: Record<string, any>;
  response: any;
}

export interface UiState {
  isRequestRunning: boolean;
  isLoadingCapabilities: boolean;
}
