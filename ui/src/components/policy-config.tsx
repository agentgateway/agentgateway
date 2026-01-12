"use client";

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import {
  ChevronDown,
  ChevronRight,
  Edit,
  Trash2,
  Plus,
  Settings,
  Loader2,
  Target,
} from "lucide-react";
import { Route as RouteType, TcpRoute, Listener, Bind } from "@/lib/types";
import { fetchBinds, updateConfig, fetchConfig } from "@/lib/api";
import { useServer } from "@/lib/server-context";
import { toast } from "sonner";
import {
  renderJwtAuthForm,
  renderCorsForm,
  renderTimeoutForm,
  renderRetryForm,
  renderDirectResponseForm,
  renderRemoteRateLimitForm,
  renderExtAuthzForm,
  renderHeaderModifierForm,
  renderBackendTLSForm,
  renderLocalRateLimitForm,
  renderMcpAuthenticationForm,
  renderMcpAuthorizationForm,
  renderBackendAuthForm,
  renderRequestRedirectForm,
  renderUrlRewriteForm,
  renderAiForm,
  renderA2aForm,
} from "@/components/policy/form-renderers";
import { POLICY_TYPES, PolicyType } from "@/lib/policy-constants";
import { getDefaultPolicyData } from "@/lib/policy-defaults";
import { useXdsMode } from "@/hooks/use-xds-mode";

interface RouteWithContext {
  route: RouteType | TcpRoute;
  listener: Listener;
  bind: Bind;
  routeIndex: number;
  routeType: "http" | "tcp";
  routeName: string;
  routePath: string;
}

interface PolicyDialogState {
  isOpen: boolean;
  type: PolicyType | null;
  route: RouteWithContext | null;
  data: any;
}

export function PolicyConfig() {
  const { refreshListeners } = useServer();
  const [expandedBinds, setExpandedBinds] = useState<Set<number>>(new Set());
  const [routes, setRoutes] = useState<RouteWithContext[]>([]);
  const [selectedRoute, setSelectedRoute] = useState<RouteWithContext | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const xds = useXdsMode();

  const [policyDialog, setPolicyDialog] = useState<PolicyDialogState>({
    isOpen: false,
    type: null,
    route: null,
    data: null,
  });

  const loadRoutes = useCallback(async () => {
    setIsLoading(true);
    try {
      const fetchedBinds = await fetchBinds();

      // Extract all routes with context
      const allRoutes: RouteWithContext[] = [];

      fetchedBinds.forEach((bind) => {
        bind.listeners.forEach((listener) => {
          // HTTP routes
          listener.routes?.forEach((route, routeIndex) => {
            const routeName = route.name || `Route ${routeIndex + 1}`;
            const routePath = route.matches?.[0]?.path
              ? route.matches[0].path.exact ||
                route.matches[0].path.pathPrefix ||
                route.matches[0].path.regex?.[0] ||
                "/*"
              : "/*";

            allRoutes.push({
              route,
              listener,
              bind,
              routeIndex,
              routeType: "http",
              routeName,
              routePath,
            });
          });

          // TCP routes
          listener.tcpRoutes?.forEach((tcpRoute, routeIndex) => {
            const routeName = tcpRoute.name || `TCP Route ${routeIndex + 1}`;
            const routePath = tcpRoute.hostnames?.join(", ") || "Any Host";

            allRoutes.push({
              route: tcpRoute,
              listener,
              bind,
              routeIndex,
              routeType: "tcp",
              routeName,
              routePath,
            });
          });
        });
      });

      setRoutes(allRoutes);

      setSelectedRoute((prev) => {
        if (!prev) return prev;
        const updatedSelectedRoute = allRoutes.find(
          (r) =>
            r.bind.port === prev.bind.port &&
            r.listener.name === prev.listener.name &&
            r.routeIndex === prev.routeIndex &&
            r.routeType === prev.routeType
        );
        return updatedSelectedRoute || prev;
      });

      // Auto-expand binds with routes
      const bindsWithRoutes = new Set<number>();
      allRoutes.forEach(({ bind }) => bindsWithRoutes.add(bind.port));
      setExpandedBinds(bindsWithRoutes);
    } catch (err) {
      console.error("Error loading routes:", err);
      toast.error("Failed to load routes");
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadRoutes();
  }, [loadRoutes]);

  const getAvailablePolicyTypes = (routeType: "http" | "tcp") => {
    return Object.entries(POLICY_TYPES)
      .filter(([_, info]) => {
        if (routeType === "http") return !info.tcpOnly;
        if (routeType === "tcp") return !info.httpOnly;
        return true;
      })
      .sort((a, b) => a[1].name.localeCompare(b[1].name));
  };

  const hasPolicyType = (routeContext: RouteWithContext, type: PolicyType) => {
    const policies = routeContext.route.policies;
    if (!policies) return false;
    return (policies as any)[type] !== undefined && (policies as any)[type] !== null;
  };

  const handleAddPolicy = async (routeContext: RouteWithContext, type: PolicyType) => {
    setPolicyDialog({
      isOpen: true,
      type,
      route: routeContext,
      data: getDefaultPolicyData(type),
    });
  };

  const handleEditPolicy = async (routeContext: RouteWithContext, type: PolicyType) => {
    const existingData = (routeContext.route.policies as any)?.[type];
    setPolicyDialog({
      isOpen: true,
      type,
      route: routeContext,
      data: existingData || getDefaultPolicyData(type),
    });
  };

  const handleDeletePolicy = async (routeContext: RouteWithContext, type: PolicyType) => {
    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find and update the route
      const bind = config.binds.find((b) => b.port === routeContext.bind.port);
      const listener = bind?.listeners.find((l) => l.name === routeContext.listener.name);

      if (routeContext.routeType === "http") {
        const route = listener?.routes?.[routeContext.routeIndex];
        if (route && route.policies) {
          delete (route.policies as any)[type];
          // Clean up empty policies object
          if (Object.keys(route.policies).length === 0) {
            delete route.policies;
          }
        }
      } else {
        const tcpRoute = listener?.tcpRoutes?.[routeContext.routeIndex];
        if (tcpRoute && tcpRoute.policies) {
          delete (tcpRoute.policies as any)[type];
          // Clean up empty policies object
          if (Object.keys(tcpRoute.policies).length === 0) {
            delete tcpRoute.policies;
          }
        }
      }

      await updateConfig(config);
      await loadRoutes();
      await refreshListeners();

      toast.success(`${POLICY_TYPES[type].name} policy deleted successfully`);
    } catch (err) {
      console.error("Error deleting policy:", err);
      toast.error("Failed to delete policy");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleSavePolicy = async () => {
    if (!policyDialog.route || !policyDialog.type) return;

    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find and update the route
      const bind = config.binds.find((b) => b.port === policyDialog.route!.bind.port);
      const listener = bind?.listeners.find((l) => l.name === policyDialog.route!.listener.name);

      if (policyDialog.route.routeType === "http") {
        const route = listener?.routes?.[policyDialog.route.routeIndex];
        if (route) {
          if (!route.policies) route.policies = {};
          (route.policies as any)[policyDialog.type] = policyDialog.data;
        }
      } else {
        const tcpRoute = listener?.tcpRoutes?.[policyDialog.route.routeIndex];
        if (tcpRoute) {
          if (!tcpRoute.policies) tcpRoute.policies = {};
          (tcpRoute.policies as any)[policyDialog.type] = policyDialog.data;
        }
      }

      await updateConfig(config);
      await loadRoutes();
      await refreshListeners();

      setPolicyDialog({
        isOpen: false,
        type: null,
        route: null,
        data: null,
      });

      toast.success(`${POLICY_TYPES[policyDialog.type].name} policy saved successfully`);
    } catch (err) {
      console.error("Error saving policy:", err);
      toast.error("Failed to save policy");
    } finally {
      setIsSubmitting(false);
    }
  };

  const getRoutesByBind = () => {
    const routesByBind = new Map<number, RouteWithContext[]>();
    routes.forEach((routeContext) => {
      const port = routeContext.bind.port;
      if (!routesByBind.has(port)) {
        routesByBind.set(port, []);
      }
      routesByBind.get(port)!.push(routeContext);
    });
    return routesByBind;
  };

  const getPolicyCount = (routeContext: RouteWithContext) => {
    const policies = routeContext.route.policies;
    if (!policies) return 0;
    return Object.keys(policies).length;
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8">
        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary"></div>
        <span className="ml-2">Loading routes...</span>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      {routes.length === 0 ? (
        <div className="text-center py-12 border rounded-md bg-muted/20">
          <Settings className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
          <p className="text-muted-foreground">No routes found.</p>
          <p className="text-sm text-muted-foreground mt-2">
            Create routes first to configure policies.
          </p>
        </div>
      ) : (
        <div className="space-y-4">
          {Array.from(getRoutesByBind().entries()).map(([port, routeContexts]) => (
            <Card key={port}>
              <Collapsible
                open={expandedBinds.has(port)}
                onOpenChange={() => {
                  setExpandedBinds((prev) => {
                    const newSet = new Set(prev);
                    if (newSet.has(port)) {
                      newSet.delete(port);
                    } else {
                      newSet.add(port);
                    }
                    return newSet;
                  });
                }}
              >
                <CollapsibleTrigger asChild>
                  <CardHeader className="hover:bg-muted/50 cursor-pointer">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center space-x-4">
                        {expandedBinds.has(port) ? (
                          <ChevronDown className="h-4 w-4" />
                        ) : (
                          <ChevronRight className="h-4 w-4" />
                        )}
                        <div>
                          <h3 className="text-lg font-semibold">Port {port}</h3>
                          <p className="text-sm text-muted-foreground">
                            {routeContexts.length} route{routeContexts.length !== 1 ? "s" : ""}{" "}
                            available
                          </p>
                        </div>
                      </div>
                      <Badge>{routeContexts.length} routes</Badge>
                    </div>
                  </CardHeader>
                </CollapsibleTrigger>

                <CollapsibleContent>
                  <CardContent className="pt-0">
                    <Table>
                      <TableHeader>
                        <TableRow>
                          <TableHead>Route</TableHead>
                          <TableHead>Type</TableHead>
                          <TableHead>Listener</TableHead>
                          <TableHead>Path/Hostnames</TableHead>
                          <TableHead>Policies</TableHead>
                          <TableHead className="text-right">Actions</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {routeContexts.map((routeContext, index) => (
                          <TableRow
                            key={index}
                            className={selectedRoute === routeContext ? "bg-muted/50" : ""}
                          >
                            <TableCell className="font-medium">{routeContext.routeName}</TableCell>
                            <TableCell>
                              <Badge
                                variant={
                                  routeContext.routeType === "http" ? "default" : "secondary"
                                }
                              >
                                {routeContext.routeType.toUpperCase()}
                              </Badge>
                            </TableCell>
                            <TableCell>
                              <Badge variant="outline">
                                {routeContext.listener.name || "Unnamed"}
                              </Badge>
                            </TableCell>
                            <TableCell className="font-mono text-sm">
                              {routeContext.routePath}
                            </TableCell>
                            <TableCell>
                              <Badge
                                variant={getPolicyCount(routeContext) > 0 ? "default" : "outline"}
                              >
                                {getPolicyCount(routeContext)} policies
                              </Badge>
                            </TableCell>
                            <TableCell className="text-right">
                              <Button
                                variant={selectedRoute === routeContext ? "default" : "outline"}
                                size="sm"
                                onClick={() => setSelectedRoute(routeContext)}
                              >
                                <Target className="h-4 w-4 mr-2" />
                                {selectedRoute === routeContext ? "Selected" : "Select"}
                              </Button>
                            </TableCell>
                          </TableRow>
                        ))}
                      </TableBody>
                    </Table>
                  </CardContent>
                </CollapsibleContent>
              </Collapsible>
            </Card>
          ))}
        </div>
      )}

      {/* Policy Configuration Section */}
      {selectedRoute && (
        <Card>
          <CardHeader>
            <div className="flex items-center justify-between">
              <div>
                <h3 className="text-lg font-semibold">Configure Policies</h3>
                <p className="text-sm text-muted-foreground">
                  {selectedRoute.routeName} ({selectedRoute.routeType.toUpperCase()}) -{" "}
                  {selectedRoute.routePath}
                </p>
              </div>
              <div className="flex items-center space-x-2">
                <Badge variant="outline">Port {selectedRoute.bind.port}</Badge>
                <Badge variant="outline">{selectedRoute.listener.name || "Unnamed"}</Badge>
              </div>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
              {getAvailablePolicyTypes(selectedRoute.routeType).map(([type, info]) => {
                const hasPolicy = hasPolicyType(selectedRoute, type as PolicyType);
                const IconComponent = info.icon;

                return (
                  <div
                    key={type}
                    className="border rounded-lg p-4 hover:bg-muted/50 transition-colors"
                  >
                    <div className="flex items-center justify-between mb-2">
                      <div className="flex items-center space-x-2">
                        <IconComponent className="h-5 w-5 text-muted-foreground" />
                        <span className="font-medium">{info.name}</span>
                      </div>
                      <Badge variant={hasPolicy ? "default" : "outline"} className="text-xs">
                        {hasPolicy ? "Active" : "Inactive"}
                      </Badge>
                    </div>
                    <p className="text-sm text-muted-foreground mb-3">{info.description}</p>
                    <div className="flex items-center space-x-2">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => {
                          if (hasPolicy) {
                            handleEditPolicy(selectedRoute, type as PolicyType);
                          } else {
                            handleAddPolicy(selectedRoute, type as PolicyType);
                          }
                        }}
                        className={`flex-1 ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
                        disabled={xds}
                      >
                        {hasPolicy ? (
                          <>
                            <Edit className="h-3 w-3 mr-2" />
                            Edit
                          </>
                        ) : (
                          <>
                            <Plus className="h-3 w-3 mr-2" />
                            Add
                          </>
                        )}
                      </Button>
                      {hasPolicy && (
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => handleDeletePolicy(selectedRoute, type as PolicyType)}
                          className={`text-destructive hover:text-destructive ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
                          disabled={isSubmitting || xds}
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </CardContent>
        </Card>
      )}

      {/* Policy Edit Dialog */}
      <Dialog
        open={policyDialog.isOpen}
        onOpenChange={(open: boolean) => {
          if (!open) {
            setPolicyDialog({
              isOpen: false,
              type: null,
              route: null,
              data: null,
            });
          }
        }}
      >
        <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>
              {policyDialog.type && POLICY_TYPES[policyDialog.type]
                ? `Configure ${POLICY_TYPES[policyDialog.type].name}`
                : "Configure Policy"}
            </DialogTitle>
            <DialogDescription>
              {policyDialog.route && (
                <div className="mt-2">
                  <Badge variant="outline" className="mr-2">
                    {policyDialog.route.routeName}
                  </Badge>
                  <Badge variant="outline" className="mr-2">
                    {policyDialog.route.routeType.toUpperCase()}
                  </Badge>
                  <span className="text-sm text-muted-foreground">
                    {policyDialog.route.routePath}
                  </span>
                </div>
              )}
              {policyDialog.type && POLICY_TYPES[policyDialog.type]
                ? POLICY_TYPES[policyDialog.type].description
                : "Configure policy settings"}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4 py-4">
            {policyDialog.type &&
              renderPolicyForm(policyDialog.type, policyDialog.data || {}, (data) => {
                setPolicyDialog((prev) => ({ ...prev, data }));
              })}
          </div>

          <DialogFooter>
            <Button
              variant="outline"
              onClick={() =>
                setPolicyDialog({
                  isOpen: false,
                  type: null,
                  route: null,
                  data: null,
                })
              }
            >
              Cancel
            </Button>
            <Button
              onClick={handleSavePolicy}
              disabled={isSubmitting || xds}
              className={xds ? "opacity-50 cursor-not-allowed" : undefined}
            >
              {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              Save Policy
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function renderPolicyForm(type: PolicyType, data: any, onChange: (data: any) => void) {
  switch (type) {
    case "jwtAuth":
      return renderJwtAuthForm({ data, onChange });

    case "cors":
      return renderCorsForm({ data, onChange });

    case "timeout":
      return renderTimeoutForm({ data, onChange });

    case "retry":
      return renderRetryForm({ data, onChange });

    case "directResponse":
      return renderDirectResponseForm({ data, onChange });

    case "remoteRateLimit":
      return renderRemoteRateLimitForm({ data, onChange });

    case "extAuthz":
      return renderExtAuthzForm({ data, onChange });

    case "requestHeaderModifier":
    case "responseHeaderModifier":
      return renderHeaderModifierForm({ data, onChange });

    case "backendTLS":
      return renderBackendTLSForm({ data, onChange });

    case "localRateLimit":
      return renderLocalRateLimitForm({ data, onChange });

    case "mcpAuthentication":
      return renderMcpAuthenticationForm({ data, onChange });

    case "mcpAuthorization":
      return renderMcpAuthorizationForm({ data, onChange });

    case "backendAuth":
      return renderBackendAuthForm({ data, onChange });

    case "requestRedirect":
      return renderRequestRedirectForm({ data, onChange });

    case "urlRewrite":
      return renderUrlRewriteForm({ data, onChange });

    case "ai":
      return renderAiForm({ data, onChange });

    case "a2a":
      return renderA2aForm({ data, onChange });

    default:
      return (
        <div className="space-y-6">
          <div className="space-y-3">
            <Label htmlFor="rawData">Raw Configuration (JSON)</Label>
            <Textarea
              id="rawData"
              value={data.__rawInput || JSON.stringify(data, null, 2)}
              onChange={(e) => {
                // Store the raw input to allow free typing
                const value = e.target.value;
                try {
                  const parsed = JSON.parse(value);
                  // Valid JSON, store the parsed data without raw input
                  onChange(parsed);
                } catch {
                  // Invalid JSON, store raw input to preserve typing
                  onChange({ ...data, __rawInput: value });
                }
              }}
              onBlur={(e) => {
                // Final validation on blur
                const value = e.target.value;
                try {
                  const parsed = JSON.parse(value);
                  onChange(parsed);
                } catch {
                  // If still invalid, revert to valid JSON
                  const cleanData = { ...data };
                  delete cleanData.__rawInput;
                  onChange(cleanData);
                }
              }}
              placeholder="{}"
              rows={8}
              className="font-mono"
            />
          </div>
        </div>
      );
  }
}
