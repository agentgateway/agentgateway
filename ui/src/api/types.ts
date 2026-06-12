/**
 * TypeScript types for AgentGateway API
 * Re-exports auto-generated types from config.d.ts
 */

// Re-export the auto-generated LocalConfig and related types from config.d.ts
export type {
  FullLocalBackend,
  LocalAIBackend,
  LocalBackendPolicies,
  LocalBind,
  LocalConfig,
  LocalListener,
  LocalListenerProtocol,
  LocalLLMConfig,
  LocalLLMModels, LocalMcpBackend, LocalMcpTarget,
  LocalPolicy,
  LocalRoute,
  LocalRouteBackend,
  LocalSimpleMcpConfig,
  LocalTCPRoute,
  LocalTCPRouteBackend,
  LocalTLSServerConfig, RouteMatch
} from "../config";

// Config Dump interface (can move this to another file later if needed)
export interface ConfigDump { 
  config?: { 
    xds?: { 
      address?: string | null;
    }
  };
  [key: string]: any; // loosely typing remaining payload for now
}
