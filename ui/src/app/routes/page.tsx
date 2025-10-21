"use client";

import { RouteConfig } from "@/components/route-config";
import { useServer } from "@/lib/server-context";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { AlertCircle, Route, AlertTriangle } from "lucide-react";
import { useState, useEffect } from "react";
import { fetchBinds } from "@/lib/api";

export default function RoutesPage() {
  const { connectionError } = useServer();
  const [isLoading, setIsLoading] = useState(true);
  const [routeStats, setRouteStats] = useState({
    totalRoutes: 0,
    totalTcpRoutes: 0,
    bindsWithRoutes: 0,
    invalidListeners: [] as Array<{ bindPort: number; listenerName: string }>,
  });

  const loadRouteStats = async () => {
    try {
      const binds = await fetchBinds();
      let totalRoutes = 0;
      let totalTcpRoutes = 0;
      let bindsWithRoutes = 0;
      const invalidListeners: Array<{ bindPort: number; listenerName: string }> = [];

      binds.forEach((bind) => {
        let bindHasRoutes = false;
        bind.listeners.forEach((listener) => {
          const hasHttpRoutes = listener.routes && listener.routes.length > 0;
          const hasTcpRoutes = listener.tcpRoutes && listener.tcpRoutes.length > 0;

          // Check for invalid configuration: both HTTP and TCP routes
          if (hasHttpRoutes && hasTcpRoutes) {
            invalidListeners.push({
              bindPort: bind.port,
              listenerName: listener.name || "unnamed",
            });
          }

          if (hasHttpRoutes) {
            totalRoutes += listener.routes!.length;
            bindHasRoutes = true;
          }
          if (hasTcpRoutes) {
            totalTcpRoutes += listener.tcpRoutes!.length;
            bindHasRoutes = true;
          }
        });
        if (bindHasRoutes) {
          bindsWithRoutes++;
        }
      });

      setRouteStats({
        totalRoutes,
        totalTcpRoutes,
        bindsWithRoutes,
        invalidListeners,
      });
    } catch (error) {
      console.error("Error loading route stats:", error);
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    loadRouteStats();
  }, []);

  return (
    <div className="container mx-auto py-8 px-4">
      <div className="flex flex-row items-center justify-between mb-6">
        <div>
          <div className="flex items-center space-x-3">
            <Route className="h-8 w-8 text-green-500" />
            <div>
              <h1 className="text-3xl font-bold tracking-tight">Routes</h1>
              <p className="text-muted-foreground mt-1">
                Configure HTTP and TCP routes with matching conditions and traffic management
              </p>
            </div>
          </div>
          {!isLoading && (routeStats.totalRoutes > 0 || routeStats.totalTcpRoutes > 0) && (
            <div className="mt-4 flex items-center space-x-6 text-sm text-muted-foreground">
              <div className="flex items-center space-x-2">
                <div className="w-2 h-2 bg-green-500 rounded-full"></div>
                <span>
                  {routeStats.totalRoutes} HTTP route{routeStats.totalRoutes !== 1 ? "s" : ""}
                </span>
              </div>
              <div className="flex items-center space-x-2">
                <div className="w-2 h-2 bg-primary rounded-full"></div>
                <span>
                  {routeStats.totalTcpRoutes} TCP route{routeStats.totalTcpRoutes !== 1 ? "s" : ""}
                </span>
              </div>
              <div className="flex items-center space-x-2">
                <div className="w-2 h-2 bg-orange-500 rounded-full"></div>
                <span>
                  {routeStats.bindsWithRoutes} bind{routeStats.bindsWithRoutes !== 1 ? "s" : ""}{" "}
                  with routes
                </span>
              </div>
            </div>
          )}
        </div>
      </div>

      {/* Invalid Configuration Warning */}
      {routeStats.invalidListeners.length > 0 && (
        <Alert variant="destructive" className="mb-6">
          <AlertTriangle className="h-4 w-4" />
          <AlertDescription>
            <div className="space-y-2">
              <div className="font-medium">Invalid Configuration Detected</div>
              <div>
                The following listeners have both HTTP and TCP routes defined, which is not
                supported. Each listener must have either HTTP routes OR TCP routes, but not both:
              </div>
              <ul className="list-disc list-inside ml-4 space-y-1">
                {routeStats.invalidListeners.map((invalid, index) => (
                  <li key={index}>
                    Port {invalid.bindPort} → Listener &quot;{invalid.listenerName}&quot;
                  </li>
                ))}
              </ul>
              <div className="text-sm">
                Please edit the configuration to remove either the HTTP routes or TCP routes from
                these listeners.
              </div>
            </div>
          </AlertDescription>
        </Alert>
      )}

      {connectionError ? (
        <Alert variant="destructive" className="mb-6">
          <AlertCircle className="h-4 w-4" />
          <AlertDescription>{connectionError}</AlertDescription>
        </Alert>
      ) : (
        <RouteConfig />
      )}
    </div>
  );
}
