"use client";

import { BackendConfig } from "@/components/backend-config";
import { useServer } from "@/lib/server-context";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { AlertCircle, Target } from "lucide-react";
import { useState, useEffect } from "react";
import { fetchBinds } from "@/lib/api";
import { getBackendType } from "@/lib/backend-utils";

export default function BackendsPage() {
  const { connectionError } = useServer();
  const [isLoading, setIsLoading] = useState(true);
  const [backendStats, setBackendStats] = useState({
    totalBackends: 0,
    mcpBackends: 0,
    aiBackends: 0,
    serviceBackends: 0,
    hostBackends: 0,
    dynamicBackends: 0,
    bindsWithBackends: 0,
  });

  const loadBackendStats = async () => {
    try {
      const binds = await fetchBinds();
      let totalBackends = 0;
      const typeCounts = {
        mcp: 0,
        ai: 0,
        service: 0,
        host: 0,
        dynamic: 0,
      };
      let bindsWithBackends = 0;

      binds.forEach((bind) => {
        let bindHasBackends = false;
        bind.listeners.forEach((listener) => {
          listener.routes?.forEach((route) => {
            if (route.backends && route.backends.length > 0) {
              totalBackends += route.backends.length;
              bindHasBackends = true;

              route.backends.forEach((backend) => {
                const type = getBackendType(backend);
                if (type in typeCounts) {
                  (typeCounts as any)[type]++;
                }
              });
            }
          });
        });
        if (bindHasBackends) {
          bindsWithBackends++;
        }
      });

      setBackendStats({
        totalBackends,
        mcpBackends: typeCounts.mcp,
        aiBackends: typeCounts.ai,
        serviceBackends: typeCounts.service,
        hostBackends: typeCounts.host,
        dynamicBackends: typeCounts.dynamic,
        bindsWithBackends,
      });
    } catch (error) {
      console.error("Error loading backend stats:", error);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadBackendStats();
  }, []);

  return (
    <div className="container mx-auto py-8 px-4">
      <div className="flex flex-row items-center justify-between mb-6">
        <div>
          <div className="flex items-center space-x-3">
            <Target className="h-8 w-8 text-primary" />
            <div>
              <h1 className="text-3xl font-bold tracking-tight">Backends</h1>
              <p className="text-muted-foreground mt-1">
                Configure backend services including MCP targets, AI providers, and service
                connections
              </p>
            </div>
          </div>
          {!isLoading && backendStats.totalBackends > 0 && (
            <div className="mt-4 flex items-center space-x-6 text-sm text-muted-foreground">
              <div className="flex items-center space-x-2">
                <div className="w-2 h-2 bg-primary rounded-full"></div>
                <span>
                  {backendStats.totalBackends} total backend
                  {backendStats.totalBackends !== 1 ? "s" : ""}
                </span>
              </div>
              {backendStats.mcpBackends > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-primary rounded-full"></div>
                  <span>{backendStats.mcpBackends} MCP</span>
                </div>
              )}
              {backendStats.aiBackends > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-green-500 rounded-full"></div>
                  <span>{backendStats.aiBackends} AI</span>
                </div>
              )}
              {backendStats.serviceBackends > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-orange-500 rounded-full"></div>
                  <span>{backendStats.serviceBackends} Service</span>
                </div>
              )}
              {backendStats.hostBackends > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-red-500 rounded-full"></div>
                  <span>{backendStats.hostBackends} Host</span>
                </div>
              )}
              {backendStats.dynamicBackends > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-yellow-500 rounded-full"></div>
                  <span>{backendStats.dynamicBackends} Dynamic</span>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      {connectionError ? (
        <Alert variant="destructive" className="mb-6">
          <AlertCircle className="h-4 w-4" />
          <AlertDescription>{connectionError}</AlertDescription>
        </Alert>
      ) : (
        <BackendConfig />
      )}
    </div>
  );
}
