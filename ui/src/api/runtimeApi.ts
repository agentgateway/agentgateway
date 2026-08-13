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
    configStoreMode: "file" | "hybrid" | "read_only";
  };
}

export function getRuntimeInfo() {
  return requestJson<RuntimeInfo>("/api/runtime");
}
