/**
 * CRUD operations for Traffic configuration
 */

import type { FilterOrPolicy } from "../config";
import { fetchConfig, updateConfig } from "./config";
import { findBindByPort, findListenerByName } from "./helpers";
import type {
  LocalBind,
  LocalListener,
  LocalRoute,
  LocalRouteBackend,
  LocalTCPRoute,
  LocalTCPRouteBackend,
} from "./types";

/**
 * Find a listener in a bind (wrapper that handles null/undefined names)
 */
function findListener(
  bind: any,
  listenerName: string | null | undefined,
): LocalListener | undefined {
  if (!listenerName) {
    return bind.listeners?.find((l: LocalListener) => !l.name);
  }
  return findListenerByName(bind, listenerName);
}

/**
 * Find a route in a listener
 */
function findRoute(
  listener: LocalListener,
  routeName: string | null | undefined,
): LocalRoute | undefined {
  return listener.routes?.find((r) => r.name === routeName);
}

// ============================================================================
// LISTENER CRUD
// ============================================================================

export interface ListenerWithPort extends LocalListener {
  port: number;
}

/**
 * Get all listeners from all binds
 */
export async function getListeners(): Promise<ListenerWithPort[]> {
  const config = await fetchConfig();
  const listeners: ListenerWithPort[] = [];

  for (const bind of config.binds || []) {
    for (const listener of bind.listeners || []) {
      listeners.push({ ...listener, port: bind.port });
    }
  }

  return listeners;
}

/**
 * Create a new listener in a bind
 */
export async function createListener(
  port: number,
  listener: LocalListener,
): Promise<void> {
  const config = await fetchConfig();

  // Find or create the bind
  let bind = findBindByPort(config.binds || [], port);
  if (!bind) {
    bind = { port, listeners: [] };
    if (!config.binds) {
      config.binds = [];
    }
    config.binds.push(bind);
  }

  // Add the listener
  if (!bind.listeners) {
    bind.listeners = [];
  }
  bind.listeners.push(listener);

  await updateConfig(config);
}

/**
 * Update an existing listener
 */
export async function updateListener(
  port: number,
  oldListenerName: string | null | undefined,
  newListener: LocalListener,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const index =
    bind.listeners?.findIndex((l) => l.name === oldListenerName) ?? -1;
  if (index === -1) {
    throw new Error(`Listener "${oldListenerName}" not found`);
  }

  if (bind.listeners) {
    bind.listeners[index] = newListener;
  }

  await updateConfig(config);
}

/**
 * Delete a listener (alternative implementation that doesn't conflict with config.ts)
 */
export async function removeListener(
  port: number,
  listenerName: string | null | undefined,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (bind.listeners) {
    bind.listeners = bind.listeners.filter((l) => l.name !== listenerName);
  }

  await updateConfig(config);
}

// ============================================================================
// ROUTE CRUD
// ============================================================================

export interface RouteWithContext extends LocalRoute {
  port: number;
  listenerName: string | null | undefined;
}

/**
 * Get all routes from all listeners
 */
export async function getRoutes(): Promise<RouteWithContext[]> {
  const config = await fetchConfig();
  const routes: RouteWithContext[] = [];

  for (const bind of config.binds || []) {
    for (const listener of bind.listeners || []) {
      for (const route of listener.routes || []) {
        routes.push({
          ...route,
          port: bind.port,
          listenerName: listener.name,
        });
      }
    }
  }

  return routes;
}

/**
 * Create a new route in a listener
 */
export async function createRoute(
  port: number,
  listenerName: string | null | undefined,
  route: LocalRoute,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  if (!listener.routes) {
    listener.routes = [];
  }
  listener.routes.push(route);

  await updateConfig(config);
}

/**
 * Update an existing route
 */
export async function updateRoute(
  port: number,
  listenerName: string | null | undefined,
  oldRouteName: string | null | undefined,
  newRoute: LocalRoute,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  const index =
    listener.routes?.findIndex((r) => r.name === oldRouteName) ?? -1;
  if (index === -1) {
    throw new Error(`Route "${oldRouteName}" not found`);
  }

  if (listener.routes) {
    listener.routes[index] = newRoute;
  }

  await updateConfig(config);
}

/**
 * Delete a route
 */
export async function deleteRoute(
  port: number,
  listenerName: string | null | undefined,
  routeName: string | null | undefined,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  if (listener.routes) {
    listener.routes = listener.routes.filter((r) => r.name !== routeName);
  }

  await updateConfig(config);
}

// ============================================================================
// BACKEND CRUD
// ============================================================================

export type BackendWithContext = LocalRouteBackend & {
  port: number;
  listenerName: string | null | undefined;
  routeName: string | null | undefined;
  index: number;
};

/**
 * Get all backends from all routes
 */
export async function getBackends(): Promise<BackendWithContext[]> {
  const config = await fetchConfig();
  const backends: BackendWithContext[] = [];

  for (const bind of config.binds || []) {
    for (const listener of bind.listeners || []) {
      for (const route of listener.routes || []) {
        for (let i = 0; i < (route.backends?.length || 0); i++) {
          const backend = route.backends![i];
          backends.push({
            ...(backend as any),
            port: bind.port,
            listenerName: listener.name,
            routeName: route.name,
            index: i,
          } as BackendWithContext);
        }
      }
    }
  }

  return backends;
}

/**
 * Create a new backend in a route
 */
export async function createBackend(
  port: number,
  listenerName: string | null | undefined,
  routeName: string | null | undefined,
  backend: LocalRouteBackend,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  const route = findRoute(listener, routeName);
  if (!route) {
    throw new Error(`Route "${routeName}" not found`);
  }

  if (!route.backends) {
    route.backends = [];
  }
  route.backends.push(backend);

  await updateConfig(config);
}

/**
 * Update an existing backend
 */
export async function updateBackend(
  port: number,
  listenerName: string | null | undefined,
  routeName: string | null | undefined,
  backendIndex: number,
  newBackend: LocalRouteBackend,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  const route = findRoute(listener, routeName);
  if (!route) {
    throw new Error(`Route "${routeName}" not found`);
  }

  if (!route.backends || backendIndex >= route.backends.length) {
    throw new Error(`Backend at index ${backendIndex} not found`);
  }

  route.backends[backendIndex] = newBackend;

  await updateConfig(config);
}

/**
 * Delete a backend
 */
export async function deleteBackend(
  port: number,
  listenerName: string | null | undefined,
  routeName: string | null | undefined,
  backendIndex: number,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  const route = findRoute(listener, routeName);
  if (!route) {
    throw new Error(`Route "${routeName}" not found`);
  }

  if (route.backends) {
    route.backends.splice(backendIndex, 1);
  }

  await updateConfig(config);
}

// ============================================================================
// POLICY CRUD (Policies are embedded in routes/listeners/backends)
// ============================================================================

export interface PolicyWithContext extends FilterOrPolicy {
  port: number;
  listenerName: string | null | undefined;
  routeName: string | null | undefined;
  policyType: "listener" | "route" | "backend";
}

/**
 * Get all policies from all routes/listeners
 */
export async function getPolicies(): Promise<PolicyWithContext[]> {
  const config = await fetchConfig();
  const policies: PolicyWithContext[] = [];

  for (const bind of config.binds || []) {
    for (const listener of bind.listeners || []) {
      // Listener policies
      if (listener.policies) {
        policies.push({
          ...listener.policies,
          port: bind.port,
          listenerName: listener.name,
          routeName: null,
          policyType: "listener",
        });
      }

      // Route policies
      for (const route of listener.routes || []) {
        if (route.policies) {
          policies.push({
            ...route.policies,
            port: bind.port,
            listenerName: listener.name,
            routeName: route.name,
            policyType: "route",
          });
        }
      }
    }
  }

  return policies;
}

/**
 * Update route policy
 */
export async function updateRoutePolicy(
  port: number,
  listenerName: string | null | undefined,
  routeName: string | null | undefined,
  policy: FilterOrPolicy,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  const route = findRoute(listener, routeName);
  if (!route) {
    throw new Error(`Route "${routeName}" not found`);
  }

  route.policies = policy;

  await updateConfig(config);
}

/**
 * Delete route policy
 */
export async function deleteRoutePolicy(
  port: number,
  listenerName: string | null | undefined,
  routeName: string | null | undefined,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  const listener = findListener(bind, listenerName);
  if (!listener) {
    throw new Error(`Listener "${listenerName}" not found`);
  }

  const route = findRoute(listener, routeName);
  if (!route) {
    throw new Error(`Route "${routeName}" not found`);
  }

  route.policies = null;

  await updateConfig(config);
}

// ============================================================================
// INDEX-BASED CRUD (for Traffic)
// Traffic uses indices instead of names for better direct access
// ============================================================================

/**
 * Update a bind by port
 */
export async function updateBind(
  port: number,
  newBind: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  const bindIndex = config.binds?.findIndex((b) => b.port === port) ?? -1;

  if (bindIndex === -1) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (config.binds) {
    config.binds[bindIndex] = newBind as unknown as LocalBind;
  }

  await updateConfig(config);
}

/**
 * Remove a bind by port
 */
export async function removeBind(port: number): Promise<void> {
  const config = await fetchConfig();

  if (config.binds) {
    config.binds = config.binds.filter((b) => b.port !== port);
  }

  await updateConfig(config);
}

/**
 * Create a new bind
 */
export async function createBind(
  newBind: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();

  if (!config.binds) {
    config.binds = [];
  }

  config.binds.push(newBind as unknown as LocalBind);

  await updateConfig(config);
}

/**
 * Create a new policy
 */
export async function createPolicy(
  newPolicy: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();

  if (!config.policies) {
    config.policies = [];
  }

  config.policies.push(newPolicy as any);

  await updateConfig(config);
}

/**
 * Create a new top-level backend
 */
export async function createTopLevelBackend(
  newBackend: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();

  if (!config.backends) {
    config.backends = [];
  }

  config.backends.push(newBackend as any);

  await updateConfig(config);
}

/**
 * Update listener by index
 */
export async function updateListenerByIndex(
  port: number,
  listenerIndex: number,
  newListener: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  bind.listeners[listenerIndex] = newListener as LocalListener;

  await updateConfig(config);
}

/**
 * Remove listener by index
 */
export async function removeListenerByIndex(
  port: number,
  listenerIndex: number,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (bind.listeners) {
    bind.listeners.splice(listenerIndex, 1);
  }

  await updateConfig(config);
}

/**
 * Update HTTP route by index
 */
export async function updateRouteByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
  newRoute: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (!listener.routes || routeIndex >= listener.routes.length) {
    throw new Error(`Route at index ${routeIndex} not found`);
  }

  listener.routes[routeIndex] = newRoute as LocalRoute;

  await updateConfig(config);
}

/**
 * Remove HTTP route by index
 */
export async function removeRouteByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (listener.routes) {
    listener.routes.splice(routeIndex, 1);
  }

  await updateConfig(config);
}

/**
 * Update TCP route by index
 */
export async function updateTCPRouteByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
  newRoute: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (!listener.tcpRoutes || routeIndex >= listener.tcpRoutes.length) {
    throw new Error(`TCP route at index ${routeIndex} not found`);
  }

  listener.tcpRoutes[routeIndex] = newRoute as LocalTCPRoute;

  await updateConfig(config);
}

/**
 * Remove TCP route by index
 */
export async function removeTCPRouteByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (listener.tcpRoutes) {
    listener.tcpRoutes.splice(routeIndex, 1);
  }

  await updateConfig(config);
}

/**
 * Update HTTP route backend by index
 */
export async function updateRouteBackendByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
  backendIndex: number,
  newBackend: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (!listener.routes || routeIndex >= listener.routes.length) {
    throw new Error(`Route at index ${routeIndex} not found`);
  }

  const route = listener.routes[routeIndex];

  if (!route.backends || backendIndex >= route.backends.length) {
    throw new Error(`Backend at index ${backendIndex} not found`);
  }

  route.backends[backendIndex] = newBackend as LocalRouteBackend;

  await updateConfig(config);
}

/**
 * Remove HTTP route backend by index
 */
export async function removeRouteBackendByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
  backendIndex: number,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (!listener.routes || routeIndex >= listener.routes.length) {
    throw new Error(`Route at index ${routeIndex} not found`);
  }

  const route = listener.routes[routeIndex];

  if (route.backends) {
    route.backends.splice(backendIndex, 1);
  }

  await updateConfig(config);
}

/**
 * Update TCP route backend by index
 */
export async function updateTCPRouteBackendByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
  backendIndex: number,
  newBackend: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (!listener.tcpRoutes || routeIndex >= listener.tcpRoutes.length) {
    throw new Error(`TCP route at index ${routeIndex} not found`);
  }

  const route = listener.tcpRoutes[routeIndex];

  if (!route.backends || backendIndex >= route.backends.length) {
    throw new Error(`Backend at index ${backendIndex} not found`);
  }

  route.backends[backendIndex] = newBackend as unknown as LocalTCPRouteBackend;

  await updateConfig(config);
}

/**
 * Remove TCP route backend by index
 */
export async function removeTCPRouteBackendByIndex(
  port: number,
  listenerIndex: number,
  routeIndex: number,
  backendIndex: number,
): Promise<void> {
  const config = await fetchConfig();
  const bind = findBindByPort(config.binds || [], port);

  if (!bind) {
    throw new Error(`Bind with port ${port} not found`);
  }

  if (!bind.listeners || listenerIndex >= bind.listeners.length) {
    throw new Error(`Listener at index ${listenerIndex} not found`);
  }

  const listener = bind.listeners[listenerIndex];

  if (!listener.tcpRoutes || routeIndex >= listener.tcpRoutes.length) {
    throw new Error(`TCP route at index ${routeIndex} not found`);
  }

  const route = listener.tcpRoutes[routeIndex];

  if (route.backends) {
    route.backends.splice(backendIndex, 1);
  }

  await updateConfig(config);
}

// ============================================================================
// TOP-LEVEL CONFIG CRUD (LLM, MCP, FRONTEND POLICIES)
// ============================================================================

/**
 * Create or update LLM configuration
 */
export async function createOrUpdateLLM(
  llmConfig: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  config.llm = llmConfig as any;
  await updateConfig(config);
}

/**
 * Delete LLM configuration
 */
export async function deleteLLM(): Promise<void> {
  const config = await fetchConfig();
  config.llm = null;
  await updateConfig(config);
}

/**
 * Create a new model in the LLM configuration
 */
export async function createLLMModel(
  modelData: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  if (!config.llm) {
    throw new Error("LLM configuration not found");
  }
  if (!Array.isArray((config.llm as any).models)) {
    (config.llm as any).models = [];
  }
  (config.llm as any).models.push(modelData);
  await updateConfig(config);
}

/**
 * Update a model by index in the LLM configuration
 */
export async function updateLLMModelByIndex(
  modelIndex: number,
  modelData: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  if (!config.llm) {
    throw new Error("LLM configuration not found");
  }
  const models = (config.llm as any).models;
  if (!Array.isArray(models) || modelIndex < 0 || modelIndex >= models.length) {
    throw new Error(`Model at index ${modelIndex} not found`);
  }
  models[modelIndex] = modelData;
  await updateConfig(config);
}

/**
 * Remove a model by index from the LLM configuration
 */
export async function removeLLMModelByIndex(modelIndex: number): Promise<void> {
  const config = await fetchConfig();
  if (!config.llm) {
    throw new Error("LLM configuration not found");
  }
  const models = (config.llm as any).models;
  if (!Array.isArray(models) || modelIndex < 0 || modelIndex >= models.length) {
    throw new Error(`Model at index ${modelIndex} not found`);
  }
  models.splice(modelIndex, 1);
  await updateConfig(config);
}

/**
 * Create or update MCP configuration
 */
export async function createOrUpdateMCP(
  mcpConfig: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  config.mcp = mcpConfig as any;
  await updateConfig(config);
}

/**
 * Delete MCP configuration
 */
export async function deleteMCP(): Promise<void> {
  const config = await fetchConfig();
  config.mcp = null;
  await updateConfig(config);
}

/**
 * Create or update Frontend Policies
 */
export async function createOrUpdateFrontendPolicies(
  policies: Record<string, unknown>,
): Promise<void> {
  const config = await fetchConfig();
  config.frontendPolicies = policies as any;
  await updateConfig(config);
}

/**
 * Delete Frontend Policies
 */
export async function deleteFrontendPolicies(): Promise<void> {
  const config = await fetchConfig();
  config.frontendPolicies = undefined;
  await updateConfig(config);
}
