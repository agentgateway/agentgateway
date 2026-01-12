"use client";

import { PolicyConfig } from "@/components/policy-config";
import { useServer } from "@/lib/server-context";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { AlertCircle, Shield } from "lucide-react";
import { useState, useEffect, useCallback } from "react";
import { fetchBinds, fetchConfig } from "@/lib/api";
import { useXdsMode } from "@/hooks/use-xds-mode";
import { AppliedPolicy } from "@/lib/types";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

export default function PoliciesPage() {
  const { connectionError } = useServer();
  const xds = useXdsMode();
  const [isLoading, setIsLoading] = useState(true);
  const [policyStats, setPolicyStats] = useState({
    totalPolicies: 0,
    securityPolicies: 0,
    trafficPolicies: 0,
    routingPolicies: 0,
    bindsWithPolicies: 0,
  });
  const [appliedPolicies, setAppliedPolicies] = useState<AppliedPolicy[]>([]);
  const [appliedLoading, setAppliedLoading] = useState<boolean>(false);
  const [appliedError, setAppliedError] = useState<string | null>(null);

  const getPolicyCategories = (policies: any) => {
    const categories = new Set<string>();

    // Security policies
    if (
      policies.jwtAuth ||
      policies.mcpAuthentication ||
      policies.mcpAuthorization ||
      policies.extAuthz
    ) {
      categories.add("security");
    }

    // Traffic policies
    if (
      policies.localRateLimit ||
      policies.remoteRateLimit ||
      policies.timeout ||
      policies.retry ||
      policies.a2a
    ) {
      categories.add("traffic");
    }

    // Routing policies
    if (
      policies.requestRedirect ||
      policies.urlRewrite ||
      policies.requestMirror ||
      policies.directResponse
    ) {
      categories.add("routing");
    }

    return Array.from(categories);
  };

  const loadPolicyStats = useCallback(async () => {
    try {
      const binds = await fetchBinds();
      let totalPolicies = 0;
      let securityPolicies = 0;
      let trafficPolicies = 0;
      let routingPolicies = 0;
      let bindsWithPolicies = 0;

      binds.forEach((bind) => {
        let bindHasPolicies = false;
        bind.listeners.forEach((listener) => {
          listener.routes?.forEach((route) => {
            if (route.policies) {
              totalPolicies++;
              bindHasPolicies = true;

              const categories = getPolicyCategories(route.policies);
              if (categories.includes("security")) securityPolicies++;
              if (categories.includes("traffic")) trafficPolicies++;
              if (categories.includes("routing")) routingPolicies++;
            }
          });
        });
        if (bindHasPolicies) {
          bindsWithPolicies++;
        }
      });

      setPolicyStats({
        totalPolicies,
        securityPolicies,
        trafficPolicies,
        routingPolicies,
        bindsWithPolicies,
      });
    } catch (error) {
      console.error("Error loading policy stats:", error);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const loadAppliedPolicies = useCallback(async () => {
    if (!xds) {
      setAppliedPolicies([]);
      return;
    }
    setAppliedLoading(true);
    setAppliedError(null);
    try {
      const cfg = await fetchConfig();
      const nonRoute = (cfg.appliedPolicies || []).filter((p) => !p.target?.route);
      setAppliedPolicies(nonRoute);
    } catch (e: any) {
      console.error("Error loading applied policies:", e);
      setAppliedError("Failed to load applied policies");
    } finally {
      setAppliedLoading(false);
    }
  }, [xds]);

  useEffect(() => {
    loadPolicyStats();
  }, [loadPolicyStats]);

  useEffect(() => {
    // Only in XDS mode do we have applied policies from config dump
    loadAppliedPolicies();
  }, [loadAppliedPolicies]);

  return (
    <div className="container mx-auto py-8 px-4">
      <div className="flex flex-row items-center justify-between mb-6">
        <div>
          <div className="flex items-center space-x-3">
            <Shield className="h-8 w-8 text-red-500" />
            <div>
              <h1 className="text-3xl font-bold tracking-tight">Policies</h1>
              <p className="text-muted-foreground mt-1">
                Configure security, traffic, and routing policies for your routes
              </p>
            </div>
          </div>
          {!isLoading && policyStats.totalPolicies > 0 && (
            <div className="mt-4 flex items-center space-x-6 text-sm text-muted-foreground">
              <div className="flex items-center space-x-2">
                <div className="w-2 h-2 bg-red-500 rounded-full"></div>
                <span>
                  {policyStats.totalPolicies} route{policyStats.totalPolicies !== 1 ? "s" : ""} with
                  policies
                </span>
              </div>
              {policyStats.securityPolicies > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-primary rounded-full"></div>
                  <span>{policyStats.securityPolicies} Security</span>
                </div>
              )}
              {policyStats.trafficPolicies > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-green-500 rounded-full"></div>
                  <span>{policyStats.trafficPolicies} Traffic</span>
                </div>
              )}
              {policyStats.routingPolicies > 0 && (
                <div className="flex items-center space-x-2">
                  <div className="w-2 h-2 bg-orange-500 rounded-full"></div>
                  <span>{policyStats.routingPolicies} Routing</span>
                </div>
              )}
            </div>
          )}
        </div>
      </div>

      {xds && (
        <div className="mb-8">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-xl font-semibold">Applied policies (non-inline)</h2>
            <Badge variant="secondary">XDS</Badge>
          </div>
          <p className="text-sm text-muted-foreground mb-4">
            These are non-inline policies that can be applied to different targets.
          </p>

          {appliedLoading ? (
            <div className="text-sm text-muted-foreground">Loading applied policies…</div>
          ) : appliedError ? (
            <Alert variant="destructive" className="mb-4">
              <AlertCircle className="h-4 w-4" />
              <AlertDescription>{appliedError}</AlertDescription>
            </Alert>
          ) : appliedPolicies.length === 0 ? (
            <Card>
              <CardContent className="py-6">
                <div className="text-sm text-muted-foreground">
                  No non-route applied policies found.
                </div>
              </CardContent>
            </Card>
          ) : (
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {appliedPolicies.map((p: AppliedPolicy) => {
                const isService = !!p.target?.backend?.service;
                const isBackend = !!p.target?.backend?.backend;
                const targetLabel = isService
                  ? `${p.target!.backend!.service!.namespace}/${p.target!.backend!.service!.hostname}`
                  : isBackend
                    ? `${p.target!.backend!.backend!.namespace}/${p.target!.backend!.backend!.name}`
                    : "Unknown target";
                return (
                  <Card key={p.key}>
                    <CardHeader className="pb-2">
                      <div className="flex items-center justify-between">
                        <div className="font-medium">
                          {p.name.kind} {p.name.namespace}/{p.name.name}
                        </div>
                        <Badge variant="outline">{isService ? "Service" : "Backend"}</Badge>
                      </div>
                      <div className="text-xs text-muted-foreground mt-1">
                        Target: {targetLabel}
                      </div>
                    </CardHeader>
                    <CardContent>
                      <div className="text-xs text-muted-foreground mb-1">Policy</div>
                      <pre className="text-xs bg-muted/50 rounded p-3 overflow-auto">
                        {JSON.stringify(p.policy, null, 2)}
                      </pre>
                    </CardContent>
                  </Card>
                );
              })}
            </div>
          )}
        </div>
      )}

      {connectionError ? (
        <Alert variant="destructive" className="mb-6">
          <AlertCircle className="h-4 w-4" />
          <AlertDescription>{connectionError}</AlertDescription>
        </Alert>
      ) : (
        <PolicyConfig />
      )}
    </div>
  );
}
