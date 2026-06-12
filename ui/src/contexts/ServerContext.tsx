import type { ReactNode } from "react";
import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";

// Server configuration and mode
export type ServerMode = "static" | "xds";

export interface ServerConfig {
  endpoint: string; // e.g., 'http://localhost:15000'
  mode: ServerMode;
  connected: boolean;
  version?: string;
}

// Statistics from dashboard
export interface ServerStats {
  listeners: number;
  routes: number;
  backends: number;
  policies: number;
  portBinds: number;
  protocols: {
    HTTP: number;
    HTTPS: number;
    TLS: number;
    TCP: number;
    HBONE: number;
  };
}

interface ServerContextType {
  config: ServerConfig;
  stats: ServerStats | null;
  updateConfig: (config: Partial<ServerConfig>) => void;
  updateStats: (stats: ServerStats) => void;
  refreshStats: () => Promise<void>;
  connect: (endpoint: string, mode: ServerMode) => Promise<boolean>;
  disconnect: () => void;
}

const defaultConfig: ServerConfig = {
  endpoint: "http://localhost:15000",
  mode: "static",
  connected: false,
};

const defaultStats: ServerStats = {
  listeners: 0,
  routes: 0,
  backends: 0,
  policies: 0,
  portBinds: 0,
  protocols: {
    HTTP: 0,
    HTTPS: 0,
    TLS: 0,
    TCP: 0,
    HBONE: 0,
  },
};

const ServerContext = createContext<ServerContextType | undefined>(undefined);

export const ServerProvider: React.FC<{ children: ReactNode }> = ({
  children,
}) => {
  const [config, setConfig] = useState<ServerConfig>(defaultConfig);
  const [stats, setStats] = useState<ServerStats | null>(null);

  const updateConfig = useCallback((newConfig: Partial<ServerConfig>) => {
    setConfig((prev) => ({ ...prev, ...newConfig }));
  }, []);

  const updateStats = useCallback((newStats: ServerStats) => {
    setStats(newStats);
  }, []);

  const refreshStats = useCallback(async () => {
    if (!config.connected) {
      return;
    }

    try {
      // TODO: Implement actual API call to fetch stats
      // For now, use default stats
      setStats(defaultStats);
    } catch (error) {
      console.error("Failed to refresh stats:", error);
    }
  }, [config.connected]);

  const connect = useCallback(
    async (endpoint: string, mode: ServerMode): Promise<boolean> => {
      try {
        // TODO: Implement actual connection logic
        // For now, simulate connection
        setConfig({
          endpoint,
          mode,
          connected: true,
          version: "1.0.0",
        });

        // Fetch initial stats
        await refreshStats();

        return true;
      } catch (error) {
        console.error("Failed to connect to server:", error);
        return false;
      }
    },
    [refreshStats],
  );

  const disconnect = useCallback(() => {
    setConfig(defaultConfig);
    setStats(null);
  }, []);

  // Load saved config from localStorage on mount
  useEffect(() => {
    const savedConfig = localStorage.getItem("serverConfig");
    if (savedConfig) {
      try {
        const parsed = JSON.parse(savedConfig);
        setConfig((prev) => ({ ...prev, ...parsed }));
      } catch (error) {
        console.error("Failed to parse saved config:", error);
      }
    }
  }, []);

  // Save config to localStorage whenever it changes
  useEffect(() => {
    localStorage.setItem("serverConfig", JSON.stringify(config));
  }, [config]);

  return (
    <ServerContext.Provider
      value={{
        config,
        stats,
        updateConfig,
        updateStats,
        refreshStats,
        connect,
        disconnect,
      }}
    >
      {children}
    </ServerContext.Provider>
  );
};

export const useServer = (): ServerContextType => {
  const context = useContext(ServerContext);
  if (!context) {
    throw new Error("useServer must be used within a ServerProvider");
  }
  return context;
};
