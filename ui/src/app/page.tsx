"use client";

import { useState, useEffect } from "react";
import { SetupWizard } from "@/components/setup-wizard";
import { useLoading } from "@/lib/loading-context";
import { useServer } from "@/lib/server-context";
import { useWizard } from "@/lib/wizard-context";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  AlertCircle,
  Network,
  Server,
  Route,
  Shield,
  ArrowRight,
  Plus,
  Globe,
  Lock,
  Zap,
  CheckCircle,
  AlertTriangle,
  Info,
  Settings,
  Code,
  Database,
} from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import Link from "next/link";
import { LoadingState } from "@/components/loading-state";
import { useXdsMode } from "@/hooks/use-xds-mode";

export default function Home() {
  const { isLoading, setIsLoading } = useLoading();
  const { setConfig, isConnected, connectionError, binds } = useServer();
  const { showWizard, handleWizardComplete, handleWizardSkip, restartWizard } = useWizard();
  const xds = useXdsMode();

  const [configUpdateMessage] = useState<{
    success: boolean;
    message: string;
  } | null>(null);

  useEffect(() => {
    if (isConnected) {
      setIsLoading(false);
    }
  }, [isConnected, setIsLoading]);

  const handleConfigChange = (newConfig: any) => {
    setConfig(newConfig);
  };

  const handleRestartWizard = async () => {
    try {
      await restartWizard();
    } catch (error) {
      console.error("Error restarting wizard:", error);
    }
  };

  // Convert Config to LocalConfig for wizard
  const localConfig = {
    binds: binds || [],
  };

  const getTotalListeners = () => {
    return binds?.reduce((total, bind) => total + bind.listeners.length, 0) || 0;
  };

  const getTotalRoutes = () => {
    let total = 0;
    binds?.forEach((bind) => {
      bind.listeners.forEach((listener) => {
        total += (listener.routes?.length || 0) + (listener.tcpRoutes?.length || 0);
      });
    });
    return total;
  };

  const getTotalBackends = () => {
    let total = 0;
    binds?.forEach((bind) => {
      bind.listeners.forEach((listener) => {
        listener.routes?.forEach((route) => {
          total += route.backends?.length || 0;
        });
        listener.tcpRoutes?.forEach((tcpRoute) => {
          total += tcpRoute.backends?.length || 0;
        });
      });
    });
    return total;
  };

  const getPolicyStats = () => {
    let totalPolicies = 0;
    let securityPolicies = 0;
    let trafficPolicies = 0;

    binds?.forEach((bind) => {
      bind.listeners.forEach((listener) => {
        listener.routes?.forEach((route) => {
          if (route.policies) {
            totalPolicies++;

            // Security policies
            if (
              route.policies.jwtAuth ||
              route.policies.mcpAuthentication ||
              route.policies.extAuthz ||
              route.policies.mcpAuthorization
            ) {
              securityPolicies++;
            }

            // Traffic policies
            if (
              route.policies.localRateLimit ||
              route.policies.remoteRateLimit ||
              route.policies.timeout ||
              route.policies.retry
            ) {
              trafficPolicies++;
            }
          }
        });
      });
    });

    return { totalPolicies, securityPolicies, trafficPolicies };
  };

  const getProtocolStats = () => {
    const protocols: { [key: string]: number } = {};
    binds?.forEach((bind) => {
      bind.listeners.forEach((listener) => {
        const protocol =
          typeof listener.protocol === "string"
            ? listener.protocol
            : listener.protocol && typeof listener.protocol === "object"
              ? Object.keys(listener.protocol as Record<string, unknown>)[0] || "HTTP"
              : "HTTP";
        protocols[protocol] = (protocols[protocol] || 0) + 1;
      });
    });
    return protocols;
  };

  const getConfigurationHealth = () => {
    const issues = [];
    const warnings = [];

    // Check for listeners without routes
    let listenersWithoutRoutes = 0;
    binds?.forEach((bind) => {
      bind.listeners.forEach((listener) => {
        const hasRoutes = (listener.routes?.length || 0) + (listener.tcpRoutes?.length || 0) > 0;
        if (!hasRoutes) {
          listenersWithoutRoutes++;
        }
      });
    });

    if (listenersWithoutRoutes > 0) {
      warnings.push(`${listenersWithoutRoutes} listener(s) without routes`);
    }

    // Check for routes without backends
    let routesWithoutBackends = 0;
    binds?.forEach((bind) => {
      bind.listeners.forEach((listener) => {
        listener.routes?.forEach((route) => {
          if (!route.backends || route.backends.length === 0) {
            routesWithoutBackends++;
          }
        });
        listener.tcpRoutes?.forEach((tcpRoute) => {
          if (!tcpRoute.backends || tcpRoute.backends.length === 0) {
            routesWithoutBackends++;
          }
        });
      });
    });

    if (routesWithoutBackends > 0) {
      issues.push(`${routesWithoutBackends} route(s) without backends`);
    }

    return { issues, warnings };
  };

  const renderContent = () => {
    if (isLoading) {
      return <LoadingState />;
    }
    if (showWizard) {
      return (
        <SetupWizard
          config={localConfig}
          onConfigChange={handleConfigChange}
          onComplete={handleWizardComplete}
          onSkip={handleWizardSkip}
        />
      );
    }

    const policyStats = getPolicyStats();
    const protocolStats = getProtocolStats();
    const healthCheck = getConfigurationHealth();

    // Show getting started if no configuration exists
    if ((binds?.length || 0) === 0) {
      return (
        <div className="text-center py-12">
          <div className="mx-auto max-w-2xl">
            <Network className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
            <h3 className="text-lg font-semibold mb-2">Welcome to agentgateway</h3>
            <p className="text-muted-foreground mb-6">
              Get started by configuring your first port bind and listener to begin routing traffic.
            </p>
            <div className="flex flex-col sm:flex-row gap-3 justify-center">
              {xds ? (
                <Button disabled className="opacity-50 cursor-not-allowed">
                  <Plus className="h-4 w-4 mr-2" />
                  Create First Listener
                </Button>
              ) : (
                <Button asChild>
                  <Link href="/listeners">
                    <Plus className="h-4 w-4 mr-2" />
                    Create First Listener
                  </Link>
                </Button>
              )}
              <Button variant="outline" onClick={handleRestartWizard} disabled={xds}>
                <Settings className="h-4 w-4 mr-2" />
                Run Setup Wizard
              </Button>
            </div>
          </div>
        </div>
      );
    }

    return (
      <div className="space-y-8">
        {/* Configuration Health Alert */}
        {(healthCheck.issues.length > 0 || healthCheck.warnings.length > 0) && (
          <div className="space-y-2">
            {healthCheck.issues.map((issue, index) => (
              <Alert key={`issue-${index}`} variant="destructive">
                <AlertTriangle className="h-4 w-4" />
                <AlertDescription>{issue}</AlertDescription>
              </Alert>
            ))}
            {healthCheck.warnings.map((warning, index) => (
              <Alert key={`warning-${index}`}>
                <Info className="h-4 w-4" />
                <AlertDescription>{warning}</AlertDescription>
              </Alert>
            ))}
          </div>
        )}

        {/* Main Statistics Cards */}
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
          <Card className="@container/card">
            <CardHeader>
              <CardDescription className="flex items-center gap-2 text-xs uppercase tracking-wider font-medium text-muted-foreground/80">
                <Network className="h-4 w-4 text-primary" />
                Port Binds
              </CardDescription>
              <CardTitle className="text-3xl font-semibold mt-2">{binds?.length || 0}</CardTitle>
              <Link
                href="/listeners"
                className="mt-3 text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 w-fit"
              >
                Manage binds
                <ArrowRight className="h-3 w-3" />
              </Link>
            </CardHeader>
          </Card>

          <Card className="@container/card">
            <CardHeader>
              <CardDescription className="flex items-center gap-2 text-xs uppercase tracking-wider font-medium text-muted-foreground/80">
                <Server className="h-4 w-4" />
                Listeners
              </CardDescription>
              <CardTitle className="text-3xl font-semibold mt-2">{getTotalListeners()}</CardTitle>
              <Link
                href="/listeners"
                className="mt-3 text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 w-fit"
              >
                View listeners
                <ArrowRight className="h-3 w-3" />
              </Link>
            </CardHeader>
          </Card>

          <Card className="@container/card">
            <CardHeader>
              <CardDescription className="flex items-center gap-2 text-xs uppercase tracking-wider font-medium text-muted-foreground/80">
                <Route className="h-4 w-4 text-orange-500" />
                Routes
              </CardDescription>
              <CardTitle className="text-3xl font-semibold mt-2">{getTotalRoutes()}</CardTitle>
              <Link
                href="/routes"
                className="mt-3 text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 w-fit"
              >
                Manage routes
                <ArrowRight className="h-3 w-3" />
              </Link>
            </CardHeader>
          </Card>

          <Card className="@container/card">
            <CardHeader>
              <CardDescription className="flex items-center gap-2 text-xs uppercase tracking-wider font-medium text-muted-foreground/80">
                <Database className="h-4 w-4 text-primary" />
                Backends
              </CardDescription>
              <CardTitle className="text-3xl font-semibold mt-2">{getTotalBackends()}</CardTitle>
              <Link
                href="/backends"
                className="mt-3 text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 w-fit"
              >
                View backends
                <ArrowRight className="h-3 w-3" />
              </Link>
            </CardHeader>
          </Card>
        </div>

        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Globe className="h-5 w-5 text-primary" />
                Protocol Distribution
              </CardTitle>
              <CardDescription>Listener protocols in use</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-3">
                {Object.entries(protocolStats).map(([protocol, count]) => (
                  <div key={protocol} className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      {protocol === "HTTPS" || protocol === "TLS" ? (
                        <Lock className="h-4 w-4" />
                      ) : (
                        <Globe className="h-4 w-4 text-primary" />
                      )}
                      <span className="text-sm font-medium">{protocol}</span>
                    </div>
                    <Badge>{count}</Badge>
                  </div>
                ))}
                {Object.keys(protocolStats).length === 0 && (
                  <p className="text-sm text-muted-foreground">No listeners configured</p>
                )}
              </div>
            </CardContent>
          </Card>

          {/* Policy Overview */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Shield className="h-5 w-5 text-red-500" />
                Policy Overview
              </CardTitle>
              <CardDescription>Security and traffic policies</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Shield className="h-4 w-4 text-red-500" />
                    <span className="text-sm font-medium">Security</span>
                  </div>
                  <Badge>{policyStats.securityPolicies}</Badge>
                </div>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Zap className="h-4 w-4 text-yellow-500" />
                    <span className="text-sm font-medium">Traffic</span>
                  </div>
                  <Badge>{policyStats.trafficPolicies}</Badge>
                </div>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <Settings className="h-4 w-4 text-gray-500" />
                    <span className="text-sm font-medium">Total Policies</span>
                  </div>
                  <Badge>{policyStats.totalPolicies}</Badge>
                </div>
                {policyStats.totalPolicies === 0 && (
                  <p className="text-sm text-muted-foreground">No policies configured</p>
                )}
              </div>
            </CardContent>
          </Card>

          {/* Quick Actions */}
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Zap className="h-5 w-5 text-yellow-500" />
                Quick Actions
              </CardTitle>
              <CardDescription>Common management tasks</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-3">
                {xds ? (
                  <Button
                    variant="outline"
                    className="w-full justify-start opacity-50 cursor-not-allowed"
                    disabled
                  >
                    <Plus className="h-4 w-4 mr-2" />
                    Add Listener
                  </Button>
                ) : (
                  <Button asChild variant="outline" className="w-full justify-start">
                    <Link href="/listeners">
                      <Plus className="h-4 w-4 mr-2" />
                      Add Listener
                    </Link>
                  </Button>
                )}
                {xds ? (
                  <Button
                    variant="outline"
                    className="w-full justify-start opacity-50 cursor-not-allowed"
                    disabled
                  >
                    <Route className="h-4 w-4 mr-2" />
                    Create Route
                  </Button>
                ) : (
                  <Button asChild variant="outline" className="w-full justify-start">
                    <Link href="/routes">
                      <Route className="h-4 w-4 mr-2" />
                      Create Route
                    </Link>
                  </Button>
                )}
                {xds ? (
                  <Button
                    variant="outline"
                    className="w-full justify-start opacity-50 cursor-not-allowed"
                    disabled
                  >
                    <Shield className="h-4 w-4 mr-2" />
                    Configure Policy
                  </Button>
                ) : (
                  <Button asChild variant="outline" className="w-full justify-start">
                    <Link href="/policies">
                      <Shield className="h-4 w-4 mr-2" />
                      Configure Policy
                    </Link>
                  </Button>
                )}
                <Button asChild variant="outline" className="w-full justify-start">
                  <Link href="/playground">
                    <Code className="h-4 w-4 mr-2" />
                    Test Routes
                  </Link>
                </Button>
              </div>
            </CardContent>
          </Card>
        </div>

        {(binds?.length || 0) > 0 && (
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <CheckCircle className="h-5 w-5" />
                Configuration Status
              </CardTitle>
              <CardDescription>Overall system health and completeness</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
                <div className="text-center">
                  <div className="text-2xl font-bold">{binds?.length || 0}</div>
                  <div className="text-sm text-muted-foreground">Active Port Binds</div>
                </div>
                <div className="text-center">
                  <div className="text-2xl font-bold text-primary">{getTotalListeners()}</div>
                  <div className="text-sm text-muted-foreground">Configured Listeners</div>
                </div>
                <div className="text-center">
                  <div className="text-2xl font-bold text-primary">{getTotalRoutes()}</div>
                  <div className="text-sm text-muted-foreground">Total Routes</div>
                </div>
              </div>
              {healthCheck.issues.length === 0 && healthCheck.warnings.length === 0 && (
                <div className="mt-4 p-3 bg-primary/10 rounded-lg">
                  <div className="flex items-center gap-2 text-primary">
                    <CheckCircle className="h-4 w-4" />
                    <span className="text-sm font-medium">Configuration looks good!</span>
                  </div>
                  <p className="text-sm  mt-1">
                    All listeners have routes and all routes have backends configured.
                  </p>
                </div>
              )}
            </CardContent>
          </Card>
        )}
      </div>
    );
  };

  return (
    <div className="container mx-auto py-8 px-4">
      {configUpdateMessage && (
        <div
          className={`mb-4 rounded-md p-4 ${configUpdateMessage.success ? "bg-green-100 text-green-800" : "bg-destructive/10 text-destructive"}`}
        >
          <p>{configUpdateMessage.message}</p>
        </div>
      )}

      {connectionError && (
        <Alert variant="destructive" className="mb-4">
          <AlertCircle className="h-4 w-4" />
          <AlertDescription>{connectionError}</AlertDescription>
        </Alert>
      )}

      {!isLoading && !showWizard && isConnected && (
        <div className="flex justify-between items-center mb-6">
          <div>
            <h1 className="text-3xl font-bold tracking-tight">Overview</h1>
            <p className="text-muted-foreground mt-1">
              Monitor your gateway&apos;s configuration and status
            </p>
          </div>
        </div>
      )}

      {renderContent()}
    </div>
  );
}
