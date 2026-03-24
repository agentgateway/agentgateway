/**
 * Configuration API functions
 */

import { mutate } from "swr";
import { get, post } from "./client";
import { cleanupConfig } from "./helpers";
import type { LocalConfig } from "./types";

/**
 * Fetches the full configuration from the agentgateway server
 */
export async function fetchConfig(): Promise<LocalConfig> {
  return get<LocalConfig>("/config");
}

/**
 * Updates the configuration and invalidates the SWR cache so all components
 * using useConfig() automatically refetch the latest data.
 */
export async function updateConfig(config: LocalConfig): Promise<void> {
  const cleanedConfig = cleanupConfig(config);
  await post<void>("/config", cleanedConfig);
  await mutate("/config");
}

/**
 * Fetches config dump (for XDS mode inspection)
 */
export async function fetchConfigDump(): Promise<any> {
  return get<any>("/config_dump");
}

/**
 * Deletes a listener by name from the specific bind
 */
export async function deleteListener(
  listenerName: string,
  port: number,
): Promise<void> {
  const config = await fetchConfig();

  // Find the bind with the matching port and remove the listener
  if (config.binds) {
    const bind = config.binds.find((b) => b.port === port);
    if (bind) {
      bind.listeners = bind.listeners.filter((listener) => {
        if (listenerName === "") {
          // Remove unnamed listeners (name is null, undefined, or empty string)
          return listener.name != null && listener.name !== "";
        } else {
          // Remove listeners with matching name
          return listener.name !== listenerName;
        }
      });
    }
  }

  await updateConfig(config);
}
