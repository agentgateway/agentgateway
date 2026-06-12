/**
 * API helper utilities
 */

import type { LocalBind, LocalConfig, LocalListener } from "./types";

/**
 * Cleans up the configuration by removing empty arrays and undefined values
 */
export function cleanupConfig(config: LocalConfig): LocalConfig {
  const cleaned = { ...config };

  // Clean up binds
  if (!cleaned.binds) return cleaned;

  cleaned.binds = cleaned.binds
    .map((bind) => {
      const cleanedBind = { ...bind };

      // Clean up listeners
      cleanedBind.listeners = cleanedBind.listeners
        .map((listener) => {
          const cleanedListener: Partial<LocalListener> = {};

          // Only include fields that have values
          if (listener.protocol) cleanedListener.protocol = listener.protocol;
          if (listener.name) cleanedListener.name = listener.name;
          if (listener.namespace) cleanedListener.namespace = listener.namespace;
          if (listener.hostname) cleanedListener.hostname = listener.hostname;
          if (listener.tls) cleanedListener.tls = listener.tls;
          if (listener.policies) cleanedListener.policies = listener.policies;

          // Include routes if non-null (even empty []). The active exclusive
          // field must be present for the API to recognise the listener mode.
          // The inactive counterpart will have been removed by the form layer.
          if (listener.routes !== null && listener.routes !== undefined) {
            cleanedListener.routes = listener.routes.map((route) => {
              const cleanedRoute: Record<string, unknown> = {
                hostnames: route.hostnames,
                matches: route.matches,
                backends: route.backends,
              };

              if (route.name) cleanedRoute.name = route.name;
              if (route.namespace) cleanedRoute.namespace = route.namespace;
              if (route.ruleName) cleanedRoute.ruleName = route.ruleName;
              if (route.policies) cleanedRoute.policies = route.policies;

              return cleanedRoute;
            });
          }

          // Include tcpRoutes if non-null (even empty [])
          if (listener.tcpRoutes !== null && listener.tcpRoutes !== undefined) {
            cleanedListener.tcpRoutes = listener.tcpRoutes.map((tcpRoute) => {
              const cleanedTCPRoute: Record<string, unknown> = {
                hostnames: tcpRoute.hostnames,
                backends: tcpRoute.backends,
              };

              if (tcpRoute.name) cleanedTCPRoute.name = tcpRoute.name;
              if (tcpRoute.namespace) cleanedTCPRoute.namespace = tcpRoute.namespace;
              if (tcpRoute.ruleName) cleanedTCPRoute.ruleName = tcpRoute.ruleName;
              if (tcpRoute.policies) cleanedTCPRoute.policies = tcpRoute.policies;

              return cleanedTCPRoute;
            });
          }

          return cleanedListener as LocalListener;
        })
        .filter((listener) => Object.keys(listener).length > 0);

      return cleanedBind;
    });

  // Clean up workloads and services - only include if they have content
  if (
    !cleaned.workloads ||
    (Array.isArray(cleaned.workloads) && cleaned.workloads.length === 0)
  ) {
    Reflect.deleteProperty(cleaned, "workloads");
  }

  if (
    !cleaned.services ||
    (Array.isArray(cleaned.services) && cleaned.services.length === 0)
  ) {
    Reflect.deleteProperty(cleaned, "services");
  }

  return cleaned;
}

/**
 * Recursively strips null values and empty arrays from an object produced
 * by RJSF.  RJSF initialises every optional array field to [] and every
 * optional scalar to null; sending those to the API can violate oneOf /
 * mutually-exclusive field constraints (e.g. routes vs tcpRoutes on a
 * listener).  This function removes them so only intentionally-set values
 * are sent.
 *
 * @param value          - The value to strip.
 * @param keepTopLevelKeys - Top-level object keys whose empty-array values
 *   should be preserved (i.e. the "active" field in a oneOf group — the API
 *   requires the field to be present even when the array is empty).
 */
export function stripFormDefaults(
  value: unknown,
  keepTopLevelKeys?: ReadonlySet<string>,
): unknown {
  if (value === null || value === undefined) return undefined;
  if (Array.isArray(value)) {
    if (value.length === 0) return undefined;
    const stripped = value
      .map((v) => stripFormDefaults(v))
      .filter((v) => v !== undefined);
    return stripped.length === 0 ? undefined : stripped;
  }
  if (typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value as Record<string, unknown>)) {
      if (keepTopLevelKeys?.has(k) && Array.isArray(v) && v.length === 0) {
        // Preserve empty array for active oneOf fields
        out[k] = [];
        continue;
      }
      const stripped = stripFormDefaults(v);
      if (stripped !== undefined) out[k] = stripped;
    }
    return out;
  }
  return value;
}

/**
 * Extracts all listeners from all binds
 */
export function extractListeners(binds: LocalBind[]): LocalListener[] {
  const allListeners: LocalListener[] = [];
  binds.forEach((bind) => {
    if (bind.listeners) {
      allListeners.push(...bind.listeners);
    }
  });
  return allListeners;
}

/**
 * Finds a bind by port number
 */
export function findBindByPort(
  binds: LocalBind[],
  port: number,
): LocalBind | undefined {
  return binds.find((bind) => bind.port === port);
}

/**
 * Finds a listener by name in a bind
 */
export function findListenerByName(
  bind: LocalBind,
  name: string,
): LocalListener | undefined {
  return bind.listeners?.find(
    (listener: LocalListener) => listener.name === name,
  );
}

/**
 * Creates a default bind structure
 */
export function createDefaultBind(port: number): LocalBind {
  return {
    port,
    listeners: [],
  };
}

/**
 * Validates port number
 */
export function isValidPort(port: number): boolean {
  return Number.isInteger(port) && port >= 1 && port <= 65535;
}

/**
 * Formats error message for display
 */
export function formatErrorMessage(error: unknown): string {
  if (error && typeof error === "object") {
    if ("message" in error) {
      return String(error.message);
    }
  }
  return String(error);
}
