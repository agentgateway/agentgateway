import { useMemo } from "react";
import { useConfig } from "../../../api";
import type {
  LocalBind,
  LocalListener,
  LocalListenerProtocol,
  LocalLLMConfig,
  LocalLLMModels,
  LocalRoute,
  LocalRouteBackend,
  LocalTCPRoute,
} from "../../../config";

// ---------------------------------------------------------------------------
// Hierarchy node types - using manual TypeScript types
// ---------------------------------------------------------------------------

export interface ValidationError {
  level: "error" | "warning";
  message: string;
}

export interface BackendNode {
  /** Raw backend object - supports both route and TCP route backends */
  backend: LocalRouteBackend | unknown;
  /** Index within route.backends */
  backendIndex: number;
  /** Whether this backend belongs to a TCP route */
  isTcpRoute: boolean;
}

export interface PolicyNode {
  /** Policy type (e.g., 'cors', 'requestHeaderModifier', 'responseHeaderModifier') */
  policyType: string;
  /** Raw policy configuration for this type */
  policy: unknown;
  /** Whether this policy belongs to a TCP route */
  isTcpRoute: boolean;
}

export interface ModelNode {
  /** Raw model data */
  model: LocalLLMModels;
  /** Index within llm.models */
  modelIndex: number;
}

export interface RouteNode {
  /** Original route data */
  route: LocalRoute | LocalTCPRoute;
  /** True when this is a TCP route (listener.tcpRoutes), false for HTTP */
  isTcp: boolean;
  /** Index within listener.routes or listener.tcpRoutes (based on isTcp) */
  categoryIndex: number;
  /** Inherited from parent bind */
  port: number;
  /** Inherited from parent listener */
  listenerName: string | null;
  listenerProtocol: LocalListenerProtocol | undefined;
  validationErrors: ValidationError[];
  /** Inline backends attached to this route */
  backends: BackendNode[];
  /** Policies attached to this route (array of policy types) */
  policies: PolicyNode[];
}

export interface ListenerNode {
  /** Original listener data */
  listener: LocalListener;
  /** Inherited from parent bind */
  port: number;
  /** Index within bind.listeners */
  listenerIndex: number;
  routes: RouteNode[];
  validationErrors: ValidationError[];
}

export interface BindNode {
  bind: LocalBind;
  listeners: ListenerNode[];
  validationErrors: ValidationError[];
}

export interface LLMNode {
  /** Raw LLM config (without models - they're in children) */
  config: Omit<LocalLLMConfig, "models">;
  /** Models defined under this LLM config */
  models: ModelNode[];
}

export interface TrafficHierarchy {
  binds: BindNode[];
  policies: unknown[];
  backends: unknown[];
  llm: LLMNode | null;
  mcp: unknown | null;
  frontendPolicies: unknown | null;
  stats: {
    totalBinds: number;
    totalListeners: number;
    totalRoutes: number;
    totalBackends: number;
    totalModels: number;
    totalValidationErrors: number;
  };
  isLoading: boolean;
  error: Error | undefined;
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

const HTTP_PROTOCOLS: Array<LocalListenerProtocol | undefined> = [
  "HTTP",
  "HTTPS",
];
const TCP_PROTOCOLS: Array<LocalListenerProtocol | undefined> = ["TCP"];

function validateRoute(
  route: LocalRoute | LocalTCPRoute,
  listener: LocalListener,
): ValidationError[] {
  const errors: ValidationError[] = [];

  // Protocol mismatch: TCP listener shouldn't have HTTP routes
  if (
    TCP_PROTOCOLS.includes(listener.protocol) &&
    "matches" in route &&
    (route.matches?.length ?? 0) > 0
  ) {
    errors.push({
      level: "warning",
      message: `Route "${route.name ?? "unnamed"}" has HTTP match conditions but is attached to a TCP listener.`,
    });
  }

  // Check if route has no backends
  if (!(route.backends?.length ?? 0)) {
    errors.push({
      level: "warning",
      message: `Route "${route.name ?? "unnamed"}" has no backends configured.`,
    });
  }

  return errors;
}

function validateListener(
  listener: LocalListener,
  bindPort: number,
  allListeners: Array<{ hostname?: string | null; port: number }>,
): ValidationError[] {
  const errors: ValidationError[] = [];

  // Duplicate hostname+port across listeners on the same bind
  if (listener.hostname && listener.hostname !== "*") {
    const duplicates = allListeners.filter(
      (l) => l.hostname === listener.hostname && l.port === bindPort,
    );
    if (duplicates.length > 1) {
      errors.push({
        level: "warning",
        message: `Hostname "${listener.hostname}" is used by multiple listeners on port ${bindPort}.`,
      });
    }
  }

  // HTTP-only protocols should not have tcpRoutes
  if (
    HTTP_PROTOCOLS.includes(listener.protocol) &&
    (listener.tcpRoutes?.length ?? 0) > 0
  ) {
    errors.push({
      level: "warning",
      message: `Listener "${listener.name ?? "unnamed"}" has TCP routes but uses protocol ${listener.protocol}.`,
    });
  }

  // Check if listener has no routes
  const hasRoutes =
    (listener.routes?.length ?? 0) > 0 || (listener.tcpRoutes?.length ?? 0) > 0;
  if (!hasRoutes) {
    errors.push({
      level: "warning",
      message: `Listener "${listener.name ?? "unnamed"}" has no routes configured.`,
    });
  }

  return errors;
}

function validateBind(bind: LocalBind): ValidationError[] {
  const errors: ValidationError[] = [];

  // Check if bind has no listeners
  if (!(bind.listeners?.length ?? 0)) {
    errors.push({
      level: "warning",
      message: `Bind on port ${bind.port} has no listeners configured.`,
    });
  }

  return errors;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

export function useTrafficHierarchy(): TrafficHierarchy {
  const { data: config, error, isLoading } = useConfig();

  return useMemo<TrafficHierarchy>(() => {
    // Flat list of all listener hostname+port pairs for duplicate detection
    const allListenerHostnames: Array<{
      hostname?: string | null;
      port: number;
    }> = (config?.binds ?? []).flatMap((bind) =>
      (bind.listeners ?? []).map((l) => ({
        hostname: l.hostname,
        port: bind.port,
      })),
    );

    let totalListeners = 0;
    let totalRoutes = 0;
    let totalBackends = 0;
    let totalModels = 0;
    let totalValidationErrors = 0;

    const binds: BindNode[] = (config?.binds ?? []).map((bind) => {
      const bindErrors = validateBind(bind);

      const listenerNodes: ListenerNode[] = (bind.listeners ?? []).map(
        (listener, listenerIndex) => {
          totalListeners++;
          const listenerErrors = validateListener(
            listener,
            bind.port,
            allListenerHostnames,
          );

          // HTTP routes
          const httpRouteNodes: RouteNode[] = (listener.routes ?? []).map(
            (route, idx) => {
              totalRoutes++;
              const routeErrors = validateRoute(route, listener);

              const backends: BackendNode[] = (route.backends ?? []).map(
                (b, bi) => {
                  totalBackends++;
                  return {
                    backend: b,
                    backendIndex: bi,
                    isTcpRoute: false,
                  };
                },
              );

              // Create a PolicyNode for each policy type in route.policies
              const policies: PolicyNode[] =
                route.policies &&
                typeof route.policies === "object" &&
                !Array.isArray(route.policies)
                  ? Object.entries(route.policies).map(
                      ([policyType, policyConfig]) => ({
                        policyType,
                        policy: policyConfig,
                        isTcpRoute: false,
                      }),
                    )
                  : [];

              return {
                route,
                isTcp: false,
                categoryIndex: idx,
                port: bind.port,
                listenerName: listener.name ?? null,
                listenerProtocol: listener.protocol,
                validationErrors: routeErrors,
                backends,
                policies,
              };
            },
          );

          // TCP routes
          const tcpRouteNodes: RouteNode[] = (listener.tcpRoutes ?? []).map(
            (route, idx) => {
              totalRoutes++;
              const routeErrors = validateRoute(route, listener);

              const backends: BackendNode[] = (route.backends ?? []).map(
                (b, bi) => {
                  totalBackends++;
                  return {
                    backend: b,
                    backendIndex: bi,
                    isTcpRoute: true,
                  };
                },
              );

              // Create a PolicyNode for each policy type in route.policies
              const policies: PolicyNode[] =
                route.policies &&
                typeof route.policies === "object" &&
                !Array.isArray(route.policies)
                  ? Object.entries(route.policies).map(
                      ([policyType, policyConfig]) => ({
                        policyType,
                        policy: policyConfig,
                        isTcpRoute: true,
                      }),
                    )
                  : [];

              return {
                route,
                isTcp: true,
                categoryIndex: idx,
                port: bind.port,
                listenerName: listener.name ?? null,
                listenerProtocol: listener.protocol,
                validationErrors: routeErrors,
                backends,
                policies,
              };
            },
          );

          const allRouteNodes = [...httpRouteNodes, ...tcpRouteNodes];

          return {
            listener,
            port: bind.port,
            listenerIndex,
            routes: allRouteNodes,
            validationErrors: listenerErrors,
          };
        },
      );

      // Count errors
      totalValidationErrors += bindErrors.length;
      for (const ln of listenerNodes) {
        totalValidationErrors += ln.validationErrors.length;
        for (const rn of ln.routes) {
          totalValidationErrors += rn.validationErrors.length;
        }
      }

      return {
        bind,
        listeners: listenerNodes,
        validationErrors: bindErrors,
      };
    });

    // Parse LLM config and models
    let llmNode: LLMNode | null = null;
    if (config?.llm) {
      const llmConfig = config.llm as LocalLLMConfig;
      const models: ModelNode[] = (llmConfig.models ?? []).map((model, idx) => {
        totalModels++;
        return {
          model,
          modelIndex: idx,
        };
      });

      // Separate models from the config
      const { models: _, ...configWithoutModels } = llmConfig;
      llmNode = {
        config: configWithoutModels,
        models,
      };
    }

    return {
      binds,
      policies: config?.policies ?? [],
      backends: config?.backends ?? [],
      llm: llmNode,
      mcp: config?.mcp ?? null,
      frontendPolicies: config?.frontendPolicies ?? null,
      stats: {
        totalBinds: binds.length,
        totalListeners,
        totalRoutes,
        totalBackends,
        totalModels,
        totalValidationErrors,
      },
      isLoading: isLoading ?? false,
      error: error as Error | undefined,
    };
  }, [config, error, isLoading]);
}
