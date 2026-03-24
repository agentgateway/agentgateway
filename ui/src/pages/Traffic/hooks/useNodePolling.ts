import { useEffect, useState } from "react";
import { useConfig } from "../../../api";
import type { UrlParams } from "../TrafficPage";
import type { useTrafficHierarchy } from "./useTrafficHierarchy";

/**
 * Hook to handle polling for a newly created node.
 * When a node is created via API and we navigate to it immediately,
 * the hierarchy might not have refreshed yet. This hook polls the
 * config until the node appears or a timeout is reached.
 */
export function useNodePolling(
  hierarchy: ReturnType<typeof useTrafficHierarchy>,
  urlParams: UrlParams,
  isCreating: boolean = false,
) {
  const { mutate } = useConfig();
  const [isPolling, setIsPolling] = useState(false);
  const [attempts, setAttempts] = useState(0);

  // Maximum number of polling attempts (20 attempts * 500ms = 10 seconds max)
  const MAX_ATTEMPTS = 20;
  const POLL_INTERVAL = 500; // milliseconds

  useEffect(() => {
    // Only start polling if we're in a "creating" state and hierarchy is loaded
    if (!isCreating || hierarchy.isLoading) {
      return;
    }

    // Check if the node exists
    const nodeExists = checkNodeExists(hierarchy, urlParams);

    if (nodeExists) {
      // Node found, stop polling
      console.log("[NodePolling] Node found, stopping polling");
      setIsPolling(false);
      setAttempts(0);
      return;
    }

    // Node doesn't exist yet - start/continue polling
    if (attempts >= MAX_ATTEMPTS) {
      // Timeout reached
      console.log("[NodePolling] Max attempts reached, giving up");
      setIsPolling(false);
      return;
    }

    console.log(
      `[NodePolling] Attempt ${attempts + 1}/${MAX_ATTEMPTS} - node not found yet`,
    );
    setIsPolling(true);

    const timeoutId = setTimeout(async () => {
      try {
        // Wait for the config to actually refresh
        await mutate();
        // Give React a tick to update the hierarchy
        await new Promise((resolve) => setTimeout(resolve, 50));
        setAttempts((prev) => prev + 1);
      } catch (error) {
        console.error("Error during polling mutate:", error);
        setAttempts((prev) => prev + 1);
      }
    }, POLL_INTERVAL);

    return () => clearTimeout(timeoutId);
  }, [isCreating, hierarchy, urlParams, attempts, mutate]);

  // Check if we've timed out
  const hasTimedOut =
    attempts >= MAX_ATTEMPTS && !checkNodeExists(hierarchy, urlParams);

  return {
    isPolling,
    hasTimedOut,
  };
}

/**
 * Check if a node exists in the hierarchy based on URL params
 */
function checkNodeExists(
  hierarchy: ReturnType<typeof useTrafficHierarchy>,
  urlParams: UrlParams,
): boolean {
  const { port, li, ri, bi, isTcpRoute, policyType, modelIndex, topLevelType } =
    urlParams;

  // Handle model nodes
  if (modelIndex !== undefined) {
    if (!hierarchy.llm) {
      console.log(`[checkNodeExists] LLM config not found`);
      return false;
    }
    const modelNode = hierarchy.llm.models[modelIndex];
    if (!modelNode) {
      console.log(`[checkNodeExists] Model not found at index ${modelIndex}`);
      return false;
    }
    console.log(`[checkNodeExists] Model found at index ${modelIndex}`);
    return true;
  }

  // Handle top-level config nodes
  if (topLevelType) {
    switch (topLevelType) {
      case "llm":
        const llmExists = !!hierarchy.llm;
        console.log(`[checkNodeExists] LLM config exists: ${llmExists}`);
        return llmExists;
      case "mcp":
        const mcpExists = !!hierarchy.mcp;
        console.log(`[checkNodeExists] MCP config exists: ${mcpExists}`);
        return mcpExists;
      case "frontendPolicies":
        const fpExists = !!hierarchy.frontendPolicies;
        console.log(`[checkNodeExists] Frontend policies exist: ${fpExists}`);
        return fpExists;
      default:
        return false;
    }
  }

  // Handle bind/listener/route/backend/policy nodes
  // These require a port
  if (port === undefined) {
    console.log(
      `[checkNodeExists] No port, modelIndex, or topLevelType - cannot check node`,
    );
    return false;
  }

  // Check bind exists
  const bindNode = hierarchy.binds.find((b) => b.bind.port === port);
  if (!bindNode) {
    console.log(`[checkNodeExists] Bind not found for port ${port}`);
    return false;
  }

  // If no listener index, we're looking at the bind itself
  if (li === undefined) return true;

  // Check listener exists
  const listenerNode = bindNode.listeners[li];
  if (!listenerNode) {
    console.log(`[checkNodeExists] Listener not found at index ${li}`);
    return false;
  }

  // If no route index, we're looking at the listener itself
  if (ri === undefined) return true;

  // Check route exists
  console.log(
    `[checkNodeExists] Looking for route: isTcp=${isTcpRoute}, categoryIndex=${ri}`,
  );
  console.log(
    `[checkNodeExists] Available routes:`,
    listenerNode.routes.map((r) => ({
      isTcp: r.isTcp,
      categoryIndex: r.categoryIndex,
    })),
  );
  const routeNode = listenerNode.routes.find(
    (rn) => rn.isTcp === isTcpRoute && rn.categoryIndex === ri,
  );
  if (!routeNode) {
    console.log(`[checkNodeExists] Route not found`);
    return false;
  }

  // If no backend index and no policy type, we're looking at the route itself
  if (bi === undefined && !policyType) return true;

  // Check policy exists
  if (policyType) {
    const policyNode = routeNode.policies.find(
      (p) => p.policyType === policyType,
    );
    return !!policyNode;
  }

  // Check backend exists
  const backendNode = routeNode.backends[bi!];
  return !!backendNode;
}
