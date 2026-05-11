/**
 * Configuration API functions
 */

import { mutate } from "swr";
import { get, post } from "./client";
import { configDumpToLocalConfig } from "./configMapper";
import { cleanupConfig } from "./helpers";
import type { ConfigDump, LocalConfig } from "./types";

async function isXdsMode(): Promise<{ xdsMode: boolean, configDump: ConfigDump }> { 
  const configDump = await fetchConfigDump();
  return {
    xdsMode: !!configDump?.config?.xds?.address,
    configDump,
  };
}

/**
 * Fetches the full configuration from the agentgateway server
 */
export async function fetchConfig(): Promise<LocalConfig> {
  // check for xDS mode
  const {xdsMode, configDump } = await isXdsMode();
  if (xdsMode) { 
    return configDumpToLocalConfig(configDump);
  }
  const data = await get<LocalConfig | null>("/config");
  return data ?? { binds: [] };
}

/**
 * Updates the configuration and invalidates the SWR cache so all components
 * using useConfig() automatically refetch the latest data.
 */
export async function updateConfig(config: LocalConfig): Promise<void> {
  // defensive check to prevent updating configuration in xDS mode
  const { xdsMode } = await isXdsMode();
  if (xdsMode) { 
    throw new Error("Cannot update configuration in xDS mode");
  }

  const cleanedConfig = cleanupConfig(config);
  await post<void>("/config", cleanedConfig);
  await mutate("/config");
}

/**
 * Fetches config dump (for XDS mode inspection)
 */
export async function fetchConfigDump(): Promise<ConfigDump> {
  return get<ConfigDump>("/config_dump");
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
