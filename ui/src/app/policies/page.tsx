"use client";

import { PolicyConfig } from "@/components/policy-config";
import { useServer } from "@/lib/server-context";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { AlertCircle, Shield } from "lucide-react";
import { useState, useEffect } from "react";
import { fetchBinds } from "@/lib/api";

export default function PoliciesPage() {
  const { connectionError } = useServer();
  const [isLoading, setIsLoading] = useState(true);
  const [policyStats, setPolicyStats] = useState({
    totalPolicies: 0,
    securityPolicies: 0,
    trafficPolicies: 0,
    routingPolicies: 0,
    bindsWithPolicies: 0,
  });

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

  const loadPolicyStats = async () => {
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
  };

  useEffect(() => {
    loadPolicyStats();
  }, []);

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
                  <div className="w-2 h-2 bg-blue-500 rounded-full"></div>
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
