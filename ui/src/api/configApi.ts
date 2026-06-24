import type { GatewayConfig } from "../types";
import { requestJson } from "./base";

export function getConfig() {
  return requestJson<GatewayConfig>("/api/config");
}

export function writeConfig(config: GatewayConfig) {
  return requestJson<{ status: string; message: string }>("/api/config", {
    method: "POST",
    body: JSON.stringify(config),
  });
}
