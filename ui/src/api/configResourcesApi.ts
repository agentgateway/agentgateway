import type {
  LlmModel,
  LlmProvider,
  LlmVirtualModel,
  VirtualApiKey,
} from "../types";
import { requestJson } from "./base";

export type ConfigResourceKind =
  | "modelCatalog"
  | "llm.provider"
  | "llm.model"
  | "llm.virtualModel"
  | "llm.apiKey"
  | "llm.policy"
  | "ui.policy";

export type PolicyResourceKind = Extract<
  ConfigResourceKind,
  `${string}.policy`
>;

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
            : K extends "llm.policy" | "ui.policy"
              ? unknown
              : never;

export interface ConfigResource<
  K extends ConfigResourceKind = ConfigResourceKind,
> {
  kind: K;
  id: string;
  value: ConfigResourceValue<K>;
  revision: number;
  createdAt: string;
  updatedAt: string;
}

export interface ConfigResourcesResponse<
  K extends ConfigResourceKind = ConfigResourceKind,
> {
  resources: ConfigResource<K>[];
}

export function listConfigResources() {
  return requestJson<ConfigResourcesResponse>("/api/config/resources");
}

export function listConfigResourcesByKind<K extends ConfigResourceKind>(
  kind: K,
) {
  return requestJson<ConfigResourcesResponse<K>>(
    `/api/config/resources/${encodeURIComponent(kind)}`,
  );
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
