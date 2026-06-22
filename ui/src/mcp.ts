import { sendMcpJsonRpc } from "./api/playgroundApi";

export type McpTool = {
  name: string;
  description?: string;
  inputSchema?: unknown;
};

export function extractMcpTools(body: unknown): McpTool[] {
  const payload = Array.isArray(body) ? body[0] : body;
  if (!payload || typeof payload !== "object") return [];
  const result = (payload as { result?: unknown }).result;
  if (!result || typeof result !== "object") return [];
  const tools = (result as { tools?: unknown }).tools;
  if (!Array.isArray(tools)) return [];
  return tools.filter(isMcpTool);
}

export async function initializeMcpSession(
  baseUrl: string,
  clientName: string,
  existingSessionId?: string,
  bearerToken?: string,
) {
  if (existingSessionId) return existingSessionId;
  const response = await sendMcpJsonRpc({
    baseUrl,
    bearerToken,
    body: {
      jsonrpc: "2.0",
      id: nextRpcId(),
      method: "initialize",
      params: {
        protocolVersion: "2025-03-26",
        capabilities: {},
        clientInfo: {
          name: clientName,
          version: "0.1.0",
        },
      },
    },
  });
  const sessionId = response.sessionId ?? "";
  await sendInitializedNotification(baseUrl, sessionId, bearerToken);
  return sessionId;
}

export async function sendInitializedNotification(
  baseUrl: string,
  sessionId?: string | null,
  bearerToken?: string,
) {
  if (!sessionId) return;
  await sendMcpJsonRpc({
    baseUrl,
    sessionId,
    bearerToken,
    body: {
      jsonrpc: "2.0",
      method: "notifications/initialized",
      params: {},
    },
  });
}

let rpcId = 0;

export function nextRpcId() {
  rpcId += 1;
  return rpcId;
}

function isMcpTool(value: unknown): value is McpTool {
  return Boolean(
    value &&
    typeof value === "object" &&
    typeof (value as { name?: unknown }).name === "string",
  );
}
