import { requestJson } from "./base";

export interface RuntimeInfo {
  build: {
    version: string;
    gitRevision: string;
    rustVersion: string;
    buildProfile: string;
    buildTarget: string;
  };
  ui: {
    gatewayMode: "standalone" | "xds";
  };
}

export function getRuntimeInfo() {
  return requestJson<RuntimeInfo>("/api/runtime");
}
