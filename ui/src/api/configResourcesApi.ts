import type {
  LlmModel,
  LlmProvider,
  LlmVirtualModel,
  McpConfig,
  McpTarget,
  TrafficGateway,
  VirtualApiKey,
} from "../types";
import type {
  LocalAttachedRoute,
  LocalAttachedTCPRoute,
} from "../gateway-config";
import { requestJson } from "./base";

export type ConfigResourceKind =
  | "modelCatalog"
  | "llm.provider"
  | "llm.model"
  | "llm.virtualModel"
  | "llm.apiKey"
  | "llm.policy"
  | "mcp.target"
  | "mcp.policy"
  | "mcp.settings"
  | "traffic.gateway"
  | "traffic.route"
  | "traffic.tcpRoute"
  | "ui.policy";

export type PolicyResourceKind = Extract<
  ConfigResourceKind,
  `${string}.policy`
>;

export type McpSettingsResource = Partial<
  Omit<McpConfig, "targets" | "policies">
>;
export type TrafficGatewayResource = TrafficGateway & { name: string };
export type TrafficRouteResource = LocalAttachedRoute & { name: string };
export type TrafficTcpRouteResource = LocalAttachedTCPRoute & { name: string };

export type ConfigResourceValue<K extends ConfigResourceKind> =
  K extends "modelCatalog"
    ? { base?: unknown; custom?: unknown }
    : K extends "llm.provider"
      ? LlmProvider
      : K extends "llm.model"
        ? LlmModel
        : K extends "llm.virtualModel"
          ? LlmVirtualModel
          : K extends "llm.apiKey"
            ? VirtualApiKey
            : K extends "mcp.target"
              ? McpTarget
              : K extends "mcp.settings"
                ? McpSettingsResource
                : K extends "traffic.gateway"
                  ? TrafficGatewayResource
                  : K extends "traffic.route"
                    ? TrafficRouteResource
                    : K extends "traffic.tcpRoute"
                      ? TrafficTcpRouteResource
                      : K extends "llm.policy" | "mcp.policy" | "ui.policy"
                        ? unknown
                        : never;

export interface ConfigResource<
  K extends ConfigResourceKind = ConfigResourceKind,
> {
  kind: K;
  id: string;
  value: ConfigResourceValue<K>;
  revision?: number;
  createdAt?: string;
  updatedAt?: string;
}

export interface ConfigResourcesResponse<
  K extends ConfigResourceKind = ConfigResourceKind,
> {
  resources: ConfigResource<K>[];
}

export function listConfigResources() {
  return requestJson<ConfigResourcesResponse>("/api/config/resources");
}

export function putConfigResources<K extends ConfigResourceKind>(
  kind: K,
  resources: ConfigResourceValue<K>[],
) {
  return requestJson<ConfigResourcesResponse<K>>(
    `/api/config/resources/${encodeURIComponent(kind)}`,
    {
      method: "PUT",
      body: JSON.stringify({
        resources: resources.map((value) => ({ value })),
      }),
    },
  );
}

export function updateConfigResource<K extends ConfigResourceKind>(
  kind: K,
  id: string,
  value: ConfigResourceValue<K>,
) {
  return requestJson<ConfigResourcesResponse<K>>(
    `/api/config/resources/${encodeURIComponent(kind)}/${encodeURIComponent(id)}`,
    {
      method: "PUT",
      body: JSON.stringify({ value }),
    },
  );
}

export function deleteConfigResource(kind: ConfigResourceKind, id: string) {
  return requestJson<{ status: string; message: string }>(
    `/api/config/resources/${encodeURIComponent(kind)}/${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
}
