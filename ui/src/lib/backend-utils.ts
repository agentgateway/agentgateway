import { Backend, Route, Listener, Bind } from "@/lib/types";
import { DEFAULT_BACKEND_FORM, BACKEND_TYPE_COLORS } from "./backend-constants";
import { POLICY_TYPES, BACKEND_POLICY_KEYS } from "./policy-constants";

/**
 * Determine the backend type based on the backend configuration
 */
export function getBackendType(backend: Backend): string {
  if (backend.mcp) return "mcp";
  if (backend.ai) return "ai";
  if (backend.service) return "service";
  if (backend.host) return "host";
  if (backend.dynamic) return "dynamic";
  return "unknown";
}

/**
 * Backend type labels for display
 */
export const BACKEND_TYPE_LABELS = {
  mcp: "MCP",
  ai: "AI",
  service: "Service",
  host: "Host",
  dynamic: "Dynamic",
  unknown: "Unknown",
} as const;

/**
 * Available AI providers
 */
export const AI_PROVIDERS = [
  { value: "openAI", label: "OpenAI" },
  { value: "gemini", label: "Gemini" },
  { value: "vertex", label: "Vertex" },
  { value: "anthropic", label: "Anthropic" },
  { value: "bedrock", label: "Bedrock" },
] as const;

/**
 * Available MCP target types
 */
export const MCP_TARGET_TYPES = [
  { value: "sse", label: "SSE" },
  { value: "mcp", label: "MCP" },
  { value: "stdio", label: "Stdio" },
  { value: "openapi", label: "OpenAPI" },
] as const;

/**
 * Backend policy keys (subset of PolicyType that apply to backends)
 */
export type BackendPolicyKey = (typeof BACKEND_POLICY_KEYS)[number];

/**
 * Default ports for different protocols
 */
export const DEFAULT_PORTS = {
  http: "80",
  https: "443",
} as const;

/**
 * Ensure a port is included in an address string
 */
export function ensurePortInAddress(
  address: string,
  defaultPort: string = DEFAULT_PORTS.http
): string {
  if (!address) return address;
  if (address.includes(":")) return address;
  return `${address}:${defaultPort}`;
}

// New utility functions extracted from backend-config.tsx

// Get backend name for display
export const getBackendName = (backend: Backend): string => {
  if (backend.mcp) {
    if (backend.mcp.targets && backend.mcp.targets.length > 0) {
      const targetNames = backend.mcp.targets.map((t) => t.name).join(", ");
      return `MCP: ${targetNames}`;
    }
    return "MCP Backend";
  }
  if (backend.ai) {
    if (backend.ai.provider) {
      const provider = Object.keys(backend.ai.provider)[0];
      const config = Object.values(backend.ai.provider)[0] as any;
      if (config?.model) {
        return `${provider.toUpperCase()}: ${config.model}`;
      }
      return `${provider.toUpperCase()} Backend`;
    }
    return "AI Backend";
  }
  if (backend.service) return backend.service.name?.hostname || "";
  if (backend.host) {
    return typeof backend.host === "string" ? backend.host : String(backend.host.name ?? "Unknown");
  }
  if (backend.dynamic) return "Dynamic Backend";
  return "Unknown Backend";
};

// Get backend type color
export const getBackendTypeColor = (type: string): string => {
  return (
    BACKEND_TYPE_COLORS[type as keyof typeof BACKEND_TYPE_COLORS] || BACKEND_TYPE_COLORS.default
  );
};

// Get backend details for table display
export const getBackendDetails = (backend: Backend): { primary: string; secondary?: string } => {
  if (backend.mcp) {
    const targetCount = `${backend.mcp.targets.length} target${backend.mcp.targets.length !== 1 ? "s" : ""}`;

    // Show details for first target if available
    if (backend.mcp.targets.length > 0) {
      const firstTarget = backend.mcp.targets[0];
      if (firstTarget.stdio) {
        const cmd = firstTarget.stdio.cmd;
        const args = firstTarget.stdio.args?.join(" ") || "";
        const fullCmd = args ? `${cmd} ${args}` : cmd;
        return {
          primary: targetCount,
          secondary: fullCmd.length > 60 ? `${fullCmd.substring(0, 60)}...` : fullCmd,
        };
      } else if (firstTarget.sse) {
        const url = `${firstTarget.sse.host}:${firstTarget.sse.port}${firstTarget.sse.path}`;
        return {
          primary: targetCount,
          secondary: url.length > 60 ? `${url.substring(0, 60)}...` : url,
        };
      } else if (firstTarget.mcp) {
        const url = `${firstTarget.mcp.host}:${firstTarget.mcp.port}${firstTarget.mcp.path}`;
        return {
          primary: targetCount,
          secondary: url.length > 60 ? `${url.substring(0, 60)}...` : url,
        };
      } else if (firstTarget.openapi) {
        const url = `${firstTarget.openapi.host}:${firstTarget.openapi.port}`;
        return {
          primary: targetCount,
          secondary: url.length > 60 ? `${url.substring(0, 60)}...` : url,
        };
      }
    }

    return { primary: targetCount };
  }

  if (backend.ai) {
    const provider = Object.keys(backend.ai.provider)[0];
    const config = Object.values(backend.ai.provider)[0] as any;
    const model = config?.model;

    return {
      primary: `Provider: ${provider}`,
      secondary: model ? `Model: ${model}` : undefined,
    };
  }

  if (backend.service) {
    return {
      primary: `Service: ${backend.service.name.hostname}`,
      secondary: `Port: ${backend.service.port}`,
    };
  }

  if (backend.host) {
    if (typeof backend.host === "string") {
      return { primary: `Address: ${backend.host}` };
    } else if (backend.host?.Hostname) {
      const hostname = String(backend.host.Hostname?.[0] ?? "");
      const port = String(backend.host.Hostname?.[1] ?? "");
      return { primary: `Host: ${hostname}`, secondary: `Port: ${port}` };
    } else if (backend.host?.Address) {
      return { primary: `Address: ${backend.host.Address}` };
    }
    return { primary: `""` };
  }

  if (backend.dynamic) {
    return { primary: "Dynamic routing" };
  }

  return { primary: "" };
};

// Form validation functions
export const validateCommonFields = (
  form: typeof DEFAULT_BACKEND_FORM,
  editingBackend?: boolean,
  backendType?: string
): boolean => {
  if (backendType !== "ai" && backendType !== "mcp" && !form.name.trim()) return false;

  // Only validate route selection when adding (not editing)
  if (!editingBackend && (!form.selectedBindPort || form.selectedRouteIndex === "")) return false;

  // Validate weight is a positive integer
  const weight = parseInt(form.weight);
  if (isNaN(weight) || weight < 0) return false;

  return true;
};

export const validateServiceBackend = (form: typeof DEFAULT_BACKEND_FORM): boolean => {
  return !!(form.serviceNamespace.trim() && form.serviceHostname.trim() && form.servicePort.trim());
};

export const validateHostBackend = (form: typeof DEFAULT_BACKEND_FORM): boolean => {
  if (form.hostType === "address") {
    return !!form.hostAddress.trim();
  } else {
    return !!(form.hostHostname.trim() && form.hostPort.trim());
  }
};

export const validateMcpBackend = (form: typeof DEFAULT_BACKEND_FORM): boolean => {
  if (form.mcpTargets.length === 0) return false;
  return form.mcpTargets.every((target) => {
    if (!target.name.trim()) return false;
    if (target.type === "stdio") {
      return !!target.cmd.trim();
    } else {
      // For SSE/MCP/OpenAPI, check if URL is provided and parsed correctly
      return !!(target.fullUrl.trim() && target.host.trim() && target.port.trim());
    }
  });
};

export const validateAiBackend = (form: typeof DEFAULT_BACKEND_FORM): boolean => {
  if (form.aiProvider === "vertex" && !form.aiProjectId.trim()) return false;
  if (form.aiProvider === "bedrock" && (!form.aiModel.trim() || !form.aiRegion.trim()))
    return false;
  return true;
};

export const validateBackendForm = (
  form: typeof DEFAULT_BACKEND_FORM,
  backendType: string,
  editingBackend?: boolean
): boolean => {
  if (!validateCommonFields(form, editingBackend, backendType)) return false;

  switch (backendType) {
    case "service":
      return validateServiceBackend(form);
    case "host":
      return validateHostBackend(form);
    case "mcp":
      return validateMcpBackend(form);
    case "ai":
      return validateAiBackend(form);
    case "dynamic":
      return true;
    default:
      return false;
  }
};

// Backend creation functions
export const addWeightIfNeeded = (backend: any, weight: number): any => {
  if (weight !== 1) backend.weight = weight;
  return backend;
};

export const createServiceBackend = (
  form: typeof DEFAULT_BACKEND_FORM,
  weight: number
): Backend => {
  return addWeightIfNeeded(
    {
      service: {
        name: {
          namespace: form.serviceNamespace,
          hostname: form.serviceHostname,
        },
        port: parseInt(form.servicePort),
      },
    },
    weight
  );
};

export const createHostBackend = (form: typeof DEFAULT_BACKEND_FORM, weight: number): Backend => {
  return addWeightIfNeeded(
    {
      host:
        form.hostType === "address"
          ? ensurePortInAddress(form.hostAddress)
          : `${form.hostHostname}:${form.hostPort || "80"}`,
    },
    weight
  );
};

export const createMcpTarget = (target: any) => {
  const baseTarget = {
    name: target.name,
  };

  switch (target.type) {
    case "sse":
      return {
        ...baseTarget,
        sse: {
          host: target.host,
          port: parseInt(target.port),
          path: target.path,
        },
      };
    case "mcp":
      return {
        ...baseTarget,
        mcp: {
          host: target.host,
          port: parseInt(target.port),
          path: target.path,
        },
      };
    case "stdio":
      return {
        ...baseTarget,
        stdio: {
          cmd: target.cmd,
          args: Array.isArray(target.args) ? target.args.filter((arg: string) => arg.trim()) : [],
          env:
            typeof target.env === "object" && target.env !== null && !Array.isArray(target.env)
              ? Object.fromEntries(
                  Object.entries(target.env).filter(
                    ([key, value]) => typeof key === "string" && key.trim() && String(value).trim()
                  )
                )
              : {},
        },
      };
    case "openapi":
      return {
        ...baseTarget,
        openapi: {
          host: target.host,
          port: parseInt(target.port),
          schema: target.schema,
        },
      };
    default:
      return baseTarget;
  }
};

export const createMcpBackend = (form: typeof DEFAULT_BACKEND_FORM, weight: number): Backend => {
  const targets = form.mcpTargets.map(createMcpTarget);
  return addWeightIfNeeded(
    {
      mcp: {
        targets,
      },
    },
    weight
  );
};

export const createAiProviderConfig = (form: typeof DEFAULT_BACKEND_FORM) => {
  const provider: any = {};

  switch (form.aiProvider) {
    case "openAI":
      provider.openAI = form.aiModel ? { model: form.aiModel } : {};
      break;
    case "gemini":
      provider.gemini = form.aiModel ? { model: form.aiModel } : {};
      break;
    case "vertex":
      provider.vertex = {
        projectId: form.aiProjectId,
        ...(form.aiModel && { model: form.aiModel }),
        ...(form.aiRegion && { region: form.aiRegion }),
      };
      break;
    case "anthropic":
      provider.anthropic = form.aiModel ? { model: form.aiModel } : {};
      break;
    case "bedrock":
      provider.bedrock = {
        model: form.aiModel,
        region: form.aiRegion,
      };
      break;
  }

  return provider;
};

export const createAiBackend = (form: typeof DEFAULT_BACKEND_FORM, weight: number): Backend => {
  const aiConfig: any = {
    provider: createAiProviderConfig(form),
  };

  // Add host override if specified
  if (form.aiHostOverrideType === "address") {
    aiConfig.hostOverride = { Address: ensurePortInAddress(form.aiHostAddress) };
  } else if (form.aiHostOverrideType === "hostname") {
    aiConfig.hostOverride = {
      Hostname: [form.aiHostHostname, parseInt(form.aiHostPort || "80")],
    };
  }

  return addWeightIfNeeded({ ai: aiConfig }, weight);
};

export const createDynamicBackend = (weight: number): Backend => {
  return addWeightIfNeeded({ dynamic: {} }, weight);
};

export const createBackendFromForm = (
  form: typeof DEFAULT_BACKEND_FORM,
  backendType: string
): Backend => {
  const weight = parseInt(form.weight) || 1;

  switch (backendType) {
    case "service":
      return createServiceBackend(form, weight);
    case "host":
      return createHostBackend(form, weight);
    case "mcp":
      return createMcpBackend(form, weight);
    case "ai":
      return createAiBackend(form, weight);
    case "dynamic":
      return createDynamicBackend(weight);
    default:
      throw new Error(`Unknown backend type: ${backendType}`);
  }
};

// Get available routes from binds
export const getAvailableRoutes = (binds: Bind[]) => {
  const routes: Array<{
    bindPort: number;
    listenerName: string;
    routeIndex: number;
    routeName: string;
    path: string;
  }> = [];

  binds.forEach((bind) => {
    bind.listeners.forEach((listener) => {
      listener.routes?.forEach((route, routeIndex) => {
        const routeName = route.name || `Route ${routeIndex + 1}`;
        const path = route.matches?.[0]?.path
          ? route.matches[0].path.exact || route.matches[0].path.pathPrefix || "/*"
          : "/*";

        routes.push({
          bindPort: bind.port,
          listenerName: listener.name || "unnamed listener",
          routeIndex,
          routeName,
          path,
        });
      });
    });
  });

  return routes;
};

// Parse and update URL for MCP targets
export const parseUrl = (url: string): { host: string; port: string; path: string } => {
  try {
    const urlObj = new URL(url);
    const host = urlObj.hostname;
    const port = urlObj.port || (urlObj.protocol === "https:" ? "443" : "80");
    const path = urlObj.pathname + urlObj.search;

    return { host, port, path };
  } catch (err) {
    // Invalid URL, return empty values
    return { host: "", port: "", path: "" };
  }
};

// Populate form from backend for editing
export const populateFormFromBackend = (
  backend: Backend,
  bind: Bind,
  listener: Listener,
  routeIndex: number
): typeof DEFAULT_BACKEND_FORM => {
  const backendType = getBackendType(backend);

  return {
    name: backendType === "ai" || backendType === "mcp" ? "" : getBackendName(backend) || "",
    weight: String(backend.weight || 1),
    selectedBindPort: String(bind.port),
    selectedListenerName: listener.name || "",
    selectedRouteIndex: String(routeIndex),

    serviceNamespace: backend.service?.name?.namespace || "",
    serviceHostname: backend.service?.name?.hostname || "",
    servicePort: String(backend.service?.port || ""),

    hostType: (() => {
      const hostStr = typeof backend.host === "string" ? backend.host : "";
      return hostStr.includes(":") ? "hostname" : "address";
    })(),
    hostAddress: typeof backend.host === "string" ? backend.host : "",
    hostHostname: (() => {
      const hostStr = typeof backend.host === "string" ? backend.host : "";
      return hostStr.includes(":") ? hostStr.split(":")[0] : "";
    })(),
    hostPort: (() => {
      const hostStr = typeof backend.host === "string" ? backend.host : "";
      return hostStr.includes(":") ? hostStr.split(":")[1] : "";
    })(),

    mcpTargets:
      backend.mcp?.targets?.map((target) => {
        const baseTarget = {
          name: target.name,
          type: "sse" as "sse" | "mcp" | "stdio" | "openapi",
          host: "",
          port: "",
          path: "",
          fullUrl: "",
          cmd: "",
          args: [] as string[],
          env: {} as Record<string, string>,
          schema: true,
        };

        if (target.sse) {
          const fullUrl = `http://${target.sse.host}:${target.sse.port}${target.sse.path}`;
          return {
            ...baseTarget,
            type: "sse" as const,
            host: target.sse.host,
            port: String(target.sse.port),
            path: target.sse.path,
            fullUrl,
          };
        } else if (target.mcp) {
          const fullUrl = `http://${target.mcp.host}:${target.mcp.port}${target.mcp.path}`;
          return {
            ...baseTarget,
            type: "mcp" as const,
            host: target.mcp.host,
            port: String(target.mcp.port),
            path: target.mcp.path,
            fullUrl,
          };
        } else if (target.stdio) {
          return {
            ...baseTarget,
            type: "stdio" as const,
            cmd: target.stdio.cmd || "",
            args: target.stdio.args || [],
            env: target.stdio.env || {},
          };
        } else if (target.openapi) {
          const fullUrl = `http://${target.openapi.host}:${target.openapi.port}`;
          return {
            ...baseTarget,
            type: "openapi" as const,
            host: target.openapi.host,
            port: String(target.openapi.port),
            path: "",
            fullUrl,
            schema: target.openapi.schema,
          };
        }
        return baseTarget;
      }) || [],
    // AI backend
    aiProvider: backend.ai?.provider ? (Object.keys(backend.ai.provider)[0] as any) : "openAI",
    aiModel: backend.ai?.provider ? Object.values(backend.ai.provider)[0]?.model || "" : "",
    aiRegion: backend.ai?.provider ? Object.values(backend.ai.provider)[0]?.region || "" : "",
    aiProjectId: backend.ai?.provider ? Object.values(backend.ai.provider)[0]?.projectId || "" : "",
    aiHostOverrideType: backend.ai?.hostOverride?.Address
      ? "address"
      : backend.ai?.hostOverride?.Hostname
        ? "hostname"
        : "none",
    aiHostAddress: backend.ai?.hostOverride?.Address || "",
    aiHostHostname: backend.ai?.hostOverride?.Hostname?.[0] || "",
    aiHostPort: String(backend.ai?.hostOverride?.Hostname?.[1] || ""),
  };
};

/**
 * Check if a route has backend policies that require exactly 1 backend
 */
export const hasBackendPolicies = (route: Route) => {
  return getBackendPolicyKeys(route).length > 0;
};

/**
 * Get the backend policy keys configured on a route
 */
export const getBackendPolicyKeys = (route: Route): BackendPolicyKey[] => {
  if (!route?.policies) return [];

  return BACKEND_POLICY_KEYS.filter(
    (policyKey) => route.policies![policyKey] !== undefined && route.policies![policyKey] !== null
  );
};

/**
 * Get the display names of backend policies configured on a route
 */
export const getBackendPolicyTypes = (route: Route): string[] => {
  const policyKeys = getBackendPolicyKeys(route);
  return policyKeys.map((key) => POLICY_TYPES[key].name);
};

/**
 * Check if a backend can be deleted based on backend policy constraints
 */
export const canDeleteBackend = (
  route: Route,
  currentBackendCount: number
): { canDelete: boolean; reason: string } => {
  if (!hasBackendPolicies(route)) {
    return { canDelete: true, reason: "" };
  }

  // If there are backend policies, we need exactly 1 backend after deletion
  const remainingBackends = currentBackendCount - 1;
  if (remainingBackends !== 1) {
    const policyTypes = getBackendPolicyTypes(route);
    const action =
      remainingBackends === 0
        ? "remove all backends"
        : remainingBackends > 1
          ? "have multiple backends"
          : "have this configuration";

    return {
      canDelete: false,
      reason:
        `Cannot ${action} when backend policies are configured. ` +
        `The following policies require exactly 1 backend: ${policyTypes.join(", ")}. ` +
        `Please remove these policies first.`,
    };
  }

  return { canDelete: true, reason: "" };
};
