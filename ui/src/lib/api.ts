import { Target, Listener, LocalConfig, Bind, Backend, Route, McpStatefulMode } from "./types";

// Mapping utilities are centralized in configMapper
import { configDumpToLocalConfig } from "./configMapper";
import {
  isXdsMode,
  ensureXdsModeLoaded,
  fetchConfigDump,
  subscribeXdsMode,
} from "@/hooks/use-xds-mode";

const API_URL = process.env.NODE_ENV === "production" ? "" : "http://localhost:15000";

let currentXdsMode = isXdsMode();
subscribeXdsMode((xdsMode) => {
  currentXdsMode = xdsMode;
});

/**
 * Fetches the full configuration from the agentgateway server
 */
export async function fetchConfig(): Promise<LocalConfig> {
  try {
    // Ensure XDS mode is determined first
    await ensureXdsModeLoaded();

    if (currentXdsMode) {
      // if xds mode is enabled, fetch the config from the configdump endpoint, since nothing is stored in the config file
      return fetchViaDump();
    } else {
      return fetchViaConfig();
    }
  } catch (error) {
    console.error("Error fetching config:", error);
    throw error;
  }

  async function fetchViaDump(): Promise<LocalConfig> {
    const dumpJson = await fetchConfigDump(API_URL);
    return configDumpToLocalConfig(dumpJson);
  }

  async function fetchViaConfig(): Promise<LocalConfig> {
    const r = await fetch(`${API_URL}/config`);
    if (!r.ok) {
      if (r.status === 500) {
        const txt = await r.text();
        const err: any = new Error(`Server configuration error: ${txt}`);
        err.isConfigurationError = true;
        err.status = 500;
        throw err;
      }
      throw new Error(`Failed to fetch config: ${r.status}`);
    }
    return (await r.json()) as LocalConfig;
  }
}

/**
 * Cleans up the configuration by removing empty arrays and undefined values
 */
function cleanupConfig(config: LocalConfig): LocalConfig {
  const cleaned = { ...config };

  // Clean up binds
  cleaned.binds = cleaned.binds.map((bind) => {
    const cleanedBind = { ...bind };

    // Clean up listeners
    cleanedBind.listeners = cleanedBind.listeners.map((listener) => {
      const cleanedListener: any = {
        protocol: listener.protocol,
      };

      // Only include fields that have values
      if (listener.name) cleanedListener.name = listener.name;
      if (listener.hostname) cleanedListener.hostname = listener.hostname;
      if (listener.tls) cleanedListener.tls = listener.tls;

      // Include routes if they exist (even if empty)
      if (listener.routes !== undefined && listener.routes !== null) {
        cleanedListener.routes = listener.routes.map((route) => {
          const cleanedRoute: any = {
            hostnames: route.hostnames,
            matches: route.matches,
            backends: route.backends,
          };

          if (route.name) cleanedRoute.name = route.name;
          if (route.ruleName) cleanedRoute.ruleName = route.ruleName;
          if (route.policies) cleanedRoute.policies = route.policies;

          return cleanedRoute;
        });
      }

      // Include tcpRoutes if they exist (even if empty)
      if (listener.tcpRoutes !== undefined && listener.tcpRoutes !== null) {
        cleanedListener.tcpRoutes = listener.tcpRoutes;
      }

      return cleanedListener;
    });

    return cleanedBind;
  });

  // Clean up workloads and services - only include if they have content
  if (cleaned.workloads && cleaned.workloads.length > 0) {
    // Keep workloads as is if they exist
  } else {
    delete (cleaned as any).workloads;
  }

  if (cleaned.services && cleaned.services.length > 0) {
    // Keep services as is if they exist
  } else {
    delete (cleaned as any).services;
  }

  return cleaned;
}

/**
 * Updates the configuration
 */
export async function updateConfig(config: LocalConfig): Promise<void> {
  if (currentXdsMode) {
    throw new Error("Configuration is managed by XDS and cannot be updated via the UI.");
  }
  try {
    // Clean up the config before sending
    const cleanedConfig = cleanupConfig(config);

    const response = await fetch(`${API_URL}/config`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(cleanedConfig),
    });

    if (!response.ok) {
      const error = await response.text();
      throw new Error(
        `Failed to update configuration: ${response.status} ${response.statusText} - ${error}`
      );
    }
  } catch (error) {
    console.error("Error updating configuration:", error);
    throw error;
  }
}

/**
 * Backward compatibility: Fetches all listeners from all binds
 */
export async function fetchListeners(): Promise<Listener[]> {
  try {
    const binds = await fetchBinds();
    const allListeners: Listener[] = [];
    binds.forEach((bind) => {
      allListeners.push(...bind.listeners);
    });
    return allListeners;
  } catch (error) {
    console.error("Error fetching listeners:", error);
    throw error;
  }
}

/**
 * Fetches all MCP targets from the agentgateway server
 */
export async function fetchMcpTargets(): Promise<any[]> {
  try {
    const config = await fetchConfig();
    const mcpTargets: any[] = [];

    config.binds.forEach((bind: Bind) => {
      bind.listeners.forEach((listener: Listener) => {
        listener.routes?.forEach((route: Route) => {
          route.backends?.forEach((backend: Backend) => {
            if (backend?.mcp) {
              mcpTargets.push(...backend.mcp.targets);
            }
          });
        });
      });
    });

    return mcpTargets;
  } catch (error) {
    console.error("Error fetching MCP targets:", error);
    throw error;
  }
}

/**
 * Fetches all A2A targets from the agentgateway server
 */
export async function fetchA2aTargets(): Promise<any[]> {
  try {
    const config = await fetchConfig();
    const a2aTargets: any[] = [];

    // Extract A2A targets from the configuration
    config.binds.forEach((bind: Bind) => {
      bind.listeners.forEach((listener: Listener) => {
        listener.routes?.forEach((route: Route) => {
          route.backends?.forEach((backend: Backend) => {
            if (backend?.ai) {
              a2aTargets.push(backend.ai);
            }
          });
        });
      });
    });

    return a2aTargets;
  } catch (error) {
    console.error("Error fetching A2A targets:", error);
    throw error;
  }
}

/**
 * Creates or updates an MCP target on the agentgateway server
 */
export async function createMcpTarget(
  target: Target,
  listenerName?: string,
  port?: number
): Promise<void> {
  try {
    const config = await fetchConfig();

    let targetBind: Bind | null = null;
    let targetListener: Listener | null = null;

    // If port is provided, find the specific bind and listener
    if (port !== undefined && listenerName !== undefined) {
      targetBind = config.binds.find((bind) => bind.port === port) || null;
      if (targetBind) {
        targetListener =
          targetBind.listeners.find((listener) => listener.name === listenerName) || null;
      }
    }

    // If no specific bind/listener found, create default structure
    if (!targetBind) {
      if (config.binds.length === 0) {
        config.binds.push({
          port: port || 8080,
          listeners: [],
        });
      }
      targetBind = config.binds[0];
    }

    if (!targetListener) {
      if (targetBind.listeners.length === 0) {
        const newListener: Listener = {
          protocol: "HTTP" as any,
        };

        // Only set fields that have values
        if (listenerName) {
          newListener.name = listenerName;
        }

        targetBind.listeners.push(newListener);
      }
      targetListener = targetBind.listeners[0];
    }

    // Ensure routes exist
    if (!targetListener.routes) {
      targetListener.routes = [];
    }

    if (targetListener.routes.length === 0) {
      targetListener.routes.push({
        hostnames: [],
        matches: [{ path: { pathPrefix: "/" } }],
        backends: [],
      });
    }

    const route = targetListener.routes[0];

    // Find or create MCP backend
    let mcpBackend = route.backends.find((backend) => backend.mcp);
    if (!mcpBackend) {
      const newMcpBackend: Backend = {
        mcp: {
          targets: [],
          statefulMode: McpStatefulMode.STATEFUL, // Default to stateful
        },
      };
      route.backends.push(newMcpBackend);
      mcpBackend = newMcpBackend;
    }

    // Add or update the target
    if (mcpBackend.mcp) {
      const existingIndex = mcpBackend.mcp.targets.findIndex((t) => t.name === target.name);

      // Build target data according to schema - only include fields with values
      const targetData: any = {
        name: target.name,
      };

      // Add the appropriate target type based on what's provided
      if (target.sse) {
        targetData.sse = target.sse;
      } else if (target.mcp) {
        targetData.mcp = target.mcp;
      } else if (target.stdio) {
        targetData.stdio = target.stdio;
      } else if (target.openapi) {
        targetData.openapi = target.openapi;
      }

      if (existingIndex >= 0) {
        mcpBackend.mcp.targets[existingIndex] = targetData;
      } else {
        mcpBackend.mcp.targets.push(targetData);
      }
    }

    await updateConfig(config);
  } catch (error) {
    console.error("Error creating MCP target:", error);
    throw error;
  }
}

/**
 * Creates or updates an A2A target on the agentgateway server
 */
export async function createA2aTarget(
  target: Target,
  listenerName?: string,
  port?: number
): Promise<void> {
  try {
    const config = await fetchConfig();

    let targetBind: Bind | null = null;
    let targetListener: Listener | null = null;

    // If port is provided, find the specific bind and listener
    if (port !== undefined && listenerName !== undefined) {
      targetBind = config.binds.find((bind) => bind.port === port) || null;
      if (targetBind) {
        targetListener =
          targetBind.listeners.find((listener) => listener.name === listenerName) || null;
      }
    }

    // If no specific bind/listener found, create default structure
    if (!targetBind) {
      if (config.binds.length === 0) {
        config.binds.push({
          port: port || 8080,
          listeners: [],
        });
      }
      targetBind = config.binds[0];
    }

    if (!targetListener) {
      if (targetBind.listeners.length === 0) {
        const newListener: Listener = {
          protocol: "HTTP" as any,
        };

        // Only set fields that have values
        if (listenerName) {
          newListener.name = listenerName;
        }

        targetBind.listeners.push(newListener);
      }
      targetListener = targetBind.listeners[0];
    }

    // Ensure routes exist
    if (!targetListener.routes) {
      targetListener.routes = [];
    }

    if (targetListener.routes.length === 0) {
      targetListener.routes.push({
        hostnames: [],
        matches: [{ path: { pathPrefix: "/" } }],
        backends: [],
      });
    }

    const route = targetListener.routes[0];

    // Create or update AI backend
    let aiBackend = route.backends.find((backend) => backend.ai);
    if (!aiBackend) {
      const newAiBackend: Backend = {
        ai: {
          name: target.name,
          provider: {
            openAI: { model: "gpt-4" }, // Default provider
          },
        },
      };

      // Only add hostOverride if a2a target has values
      if (target.a2a) {
        newAiBackend.ai!.hostOverride = `${target.a2a.host}:${target.a2a.port}`;
      }

      route.backends.push(newAiBackend);
      aiBackend = newAiBackend;
    } else {
      // Update existing AI backend
      if (aiBackend.ai) {
        aiBackend.ai.name = target.name;

        // Only set hostOverride if a2a target has values
        if (target.a2a) {
          aiBackend.ai.hostOverride = `${target.a2a.host}:${target.a2a.port}`;
        } else {
          // Remove hostOverride if no a2a config
          delete aiBackend.ai.hostOverride;
        }
      }
    }

    await updateConfig(config);
  } catch (error) {
    console.error("Error creating A2A target:", error);
    throw error;
  }
}

/**
 * Updates a single target on the agentgateway server
 */
export async function updateTarget(
  target: Target,
  listenerName?: string,
  port?: number
): Promise<void> {
  try {
    if (target.sse || target.mcp || target.stdio || target.openapi) {
      await createMcpTarget(target, listenerName, port);
    } else if (target.a2a) {
      await createA2aTarget(target, listenerName, port);
    } else {
      throw new Error("Invalid target type");
    }
  } catch (error) {
    console.error("Error updating target:", error);
    throw error;
  }
}

/**
 * Fetches a specific MCP target by name
 */
export async function getMcpTarget(name: string): Promise<any> {
  try {
    const mcpTargets = await fetchMcpTargets();
    const target = mcpTargets.find((t) => t.name === name);

    if (!target) {
      throw new Error(`MCP target '${name}' not found`);
    }

    return target;
  } catch (error) {
    console.error("Error fetching MCP target:", error);
    throw error;
  }
}

/**
 * Fetches a specific A2A target by name
 */
export async function getA2aTarget(name: string): Promise<any> {
  try {
    const a2aTargets = await fetchA2aTargets();
    const target = a2aTargets.find((t) => t.name === name);

    if (!target) {
      throw new Error(`A2A target '${name}' not found`);
    }

    return target;
  } catch (error) {
    console.error("Error fetching A2A target:", error);
    throw error;
  }
}

/**
 * Deletes an MCP target by name
 */
export async function deleteMcpTarget(name: string): Promise<void> {
  try {
    const config = await fetchConfig();

    // Find and remove the target from all MCP backends
    config.binds.forEach((bind: Bind) => {
      bind.listeners.forEach((listener: Listener) => {
        listener.routes?.forEach((route: Route) => {
          route.backends.forEach((backend: Backend) => {
            if (backend.mcp) {
              backend.mcp.targets = backend.mcp.targets.filter((t) => t.name !== name);
            }
          });
        });
      });
    });

    await updateConfig(config);
  } catch (error) {
    console.error("Error deleting MCP target:", error);
    throw error;
  }
}

/**
 * Deletes an A2A target by name
 */
export async function deleteA2aTarget(name: string): Promise<void> {
  try {
    const config = await fetchConfig();

    // Find and remove the A2A backend
    config.binds.forEach((bind: Bind) => {
      bind.listeners.forEach((listener: Listener) => {
        listener.routes?.forEach((route: Route) => {
          route.backends = route.backends.filter(
            (backend) => !backend.ai || backend.ai.name !== name
          );
        });
      });
    });

    await updateConfig(config);
  } catch (error) {
    console.error("Error deleting A2A target:", error);
    throw error;
  }
}

/**
 * Fetches targets associated with a specific listener
 */
export async function fetchListenerTargets(listenerName: string): Promise<any[]> {
  try {
    const config = await fetchConfig();
    const targets: any[] = [];

    config.binds.forEach((bind: Bind) => {
      bind.listeners.forEach((listener: Listener) => {
        if (listener.name === listenerName) {
          listener.routes?.forEach((route: Route) => {
            route.backends.forEach((backend: Backend) => {
              if (backend.mcp) {
                targets.push(...backend.mcp.targets);
              }
              if (backend.ai) {
                targets.push(backend.ai);
              }
            });
          });
        }
      });
    });

    return targets;
  } catch (error) {
    console.error("Error fetching listener targets:", error);
    throw error;
  }
}

/**
 * Fetches a specific listener by name
 */
export async function getListener(name: string): Promise<Listener> {
  try {
    const listeners = await fetchListeners();
    const listener = listeners.find((l) => l.name === name);

    if (!listener) {
      throw new Error(`Listener '${name}' not found`);
    }

    return listener;
  } catch (error) {
    console.error("Error fetching listener:", error);
    throw error;
  }
}

/**
 * Creates or updates a listener on the agentgateway server
 */
export async function createListener(listener: Listener, port?: number): Promise<void> {
  try {
    const config = await fetchConfig();

    // Use provided port or default
    const targetPort = port || 8080;

    // Find or create a bind for the specified port
    let bind = config.binds.find((b) => b.port === targetPort);
    if (!bind) {
      bind = {
        port: targetPort,
        listeners: [],
      };
      config.binds.push(bind);
    }

    // Add or update the listener
    const existingIndex = bind.listeners.findIndex((l) => l.name === listener.name);
    if (existingIndex >= 0) {
      bind.listeners[existingIndex] = listener;
    } else {
      bind.listeners.push(listener);
    }

    await updateConfig(config);
  } catch (error) {
    console.error("Error creating listener:", error);
    throw error;
  }
}

/**
 * Backward compatibility: Adds a listener (wraps addListenerToBind)
 */
export async function addListener(listener: Listener, port: number): Promise<void> {
  return addListenerToBind(listener, port);
}

/**
 * Backward compatibility: Deletes a listener (wraps removeListenerFromBind)
 */
export async function deleteListener(listener: Listener): Promise<void> {
  if (!listener.name) {
    throw new Error("Listener name is required for deletion");
  }
  return removeListenerFromBind(listener.name);
}

/**
 * Deletes all targets and listeners from the agentgateway server
 */
export async function deleteEverything(): Promise<void> {
  try {
    const config = await fetchConfig();

    // Clear all binds (which contain listeners and their targets)
    config.binds = [];

    await updateConfig(config);
  } catch (error) {
    console.error("Error deleting everything:", error);
    throw error;
  }
}

/**
 * Fetches all binds from the agentgateway server
 */
export async function fetchBinds(): Promise<Bind[]> {
  try {
    const config = await fetchConfig();
    return config.binds || [];
  } catch (error) {
    console.error("Error fetching binds:", error);
    throw error;
  }
}

/**
 * Creates a new bind (port binding) on the agentgateway server
 */
export async function createBind(port: number): Promise<void> {
  try {
    const config = await fetchConfig();

    // Check if bind already exists
    const existingBind = config.binds.find((b) => b.port === port);
    if (existingBind) {
      throw new Error(`Bind for port ${port} already exists`);
    }

    // Add new bind
    const newBind: Bind = {
      port,
      listeners: [],
    };

    config.binds.push(newBind);
    await updateConfig(config);
  } catch (error) {
    console.error("Error creating bind:", error);
    throw error;
  }
}

/**
 * Deletes a bind and all its listeners
 */
export async function deleteBind(port: number): Promise<void> {
  try {
    const config = await fetchConfig();

    // Remove the bind
    config.binds = config.binds.filter((bind) => bind.port !== port);

    await updateConfig(config);
  } catch (error) {
    console.error("Error deleting bind:", error);
    throw error;
  }
}

/**
 * Adds a listener to a specific bind
 */
export async function addListenerToBind(listener: Listener, port: number): Promise<void> {
  try {
    const config = await fetchConfig();

    // Find the bind
    let bind = config.binds.find((b) => b.port === port);
    if (!bind) {
      // Create bind if it doesn't exist
      bind = {
        port,
        listeners: [],
      };
      config.binds.push(bind);
    }

    // Check if listener name already exists in this bind
    const existingIndex = bind.listeners.findIndex((l) => l.name === listener.name);
    if (existingIndex >= 0) {
      bind.listeners[existingIndex] = listener;
    } else {
      bind.listeners.push(listener);
    }

    await updateConfig(config);
  } catch (error) {
    console.error("Error adding listener to bind:", error);
    throw error;
  }
}

/**
 * Removes a listener from its bind
 */
export async function removeListenerFromBind(listenerName: string): Promise<void> {
  try {
    const config = await fetchConfig();

    // Find and remove the listener from all binds
    config.binds.forEach((bind: Bind) => {
      bind.listeners = bind.listeners.filter((l) => l.name !== listenerName);
    });

    // Remove empty binds (optional - you might want to keep empty binds)
    // config.binds = config.binds.filter(bind => bind.listeners.length > 0);

    await updateConfig(config);
  } catch (error) {
    console.error("Error removing listener from bind:", error);
    throw error;
  }
}

/**
 * Moves a listener from one bind to another
 */
export async function moveListenerToBind(
  listenerName: string,
  fromPort: number,
  toPort: number
): Promise<void> {
  try {
    const config = await fetchConfig();

    // Find the listener in the source bind
    const sourceBind = config.binds.find((b) => b.port === fromPort);
    if (!sourceBind) {
      throw new Error(`Source bind for port ${fromPort} not found`);
    }

    const listenerIndex = sourceBind.listeners.findIndex((l) => l.name === listenerName);
    if (listenerIndex === -1) {
      throw new Error(`Listener ${listenerName} not found in port ${fromPort}`);
    }

    const listener = sourceBind.listeners[listenerIndex];

    // Remove from source bind
    sourceBind.listeners.splice(listenerIndex, 1);

    // Add to target bind
    let targetBind = config.binds.find((b) => b.port === toPort);
    if (!targetBind) {
      // Create target bind if it doesn't exist
      targetBind = {
        port: toPort,
        listeners: [],
      };
      config.binds.push(targetBind);
    }

    targetBind.listeners.push(listener);

    await updateConfig(config);
  } catch (error) {
    console.error("Error moving listener between binds:", error);
    throw error;
  }
}

/**
 * Gets bind information for a specific port
 */
export async function getBind(port: number): Promise<Bind | null> {
  try {
    const config = await fetchConfig();
    return config.binds.find((b) => b.port === port) || null;
  } catch (error) {
    console.error("Error getting bind:", error);
    return null;
  }
}
