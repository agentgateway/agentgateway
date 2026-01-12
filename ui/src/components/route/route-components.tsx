import React from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  ChevronDown,
  ChevronRight,
  Trash2,
  Edit,
  Globe,
  Server,
  Loader2,
  Network,
} from "lucide-react";
import { Route as RouteType, TcpRoute, Listener, Bind } from "@/lib/types";
import { CombinedRouteWithContext, RouteWithContext, TcpRouteWithContext } from "@/lib/route-hooks";
import {
  DEFAULT_HTTP_ROUTE_FORM,
  DEFAULT_TCP_ROUTE_FORM,
  PATH_MATCH_TYPES,
  ROUTE_TABLE_HEADERS,
  ROUTE_TYPE_CONFIGS,
} from "@/lib/route-constants";
import { isTcpListener, getPathDisplayString } from "@/lib/route-utils";
import { ListenerProtocol } from "@/lib/types";

interface RouteTableProps {
  allRoutesByBind: Map<number, CombinedRouteWithContext[]>;
  expandedBinds: Set<number>;
  setExpandedBinds: React.Dispatch<React.SetStateAction<Set<number>>>;
  onEditRoute: (route: RouteWithContext) => void;
  onEditTcpRoute: (tcpRoute: TcpRouteWithContext) => void;
  onDeleteRoute: (route: RouteWithContext) => void;
  onDeleteTcpRoute: (tcpRoute: TcpRouteWithContext) => void;
  xds: boolean;
}

export const RouteTable: React.FC<RouteTableProps> = ({
  allRoutesByBind,
  expandedBinds,
  setExpandedBinds,
  onEditRoute,
  onEditTcpRoute,
  onDeleteRoute,
  onDeleteTcpRoute,
  xds,
}) => {
  return (
    <div className="space-y-4">
      {Array.from(allRoutesByBind.entries()).map(([port, combinedRoutes]) => {
        if (combinedRoutes.length === 0) return null;

        const httpCount = combinedRoutes.filter((r) => r.type === "http").length;
        const tcpCount = combinedRoutes.filter((r) => r.type === "tcp").length;

        return (
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
                        <div className="flex items-center space-x-4 text-sm text-muted-foreground mt-1">
                          {httpCount > 0 && (
                            <div className="flex items-center space-x-1">
                              <Globe className="h-3 w-3 text-green-500" />
                              <span>{httpCount} HTTP</span>
                            </div>
                          )}
                          {tcpCount > 0 && (
                            <div className="flex items-center space-x-1">
                              <Server className="h-3 w-3 text-primary" />
                              <span>{tcpCount} TCP</span>
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                    <Badge>{combinedRoutes.length} routes</Badge>
                  </div>
                </CardHeader>
              </CollapsibleTrigger>

              <CollapsibleContent>
                <CardContent className="pt-0">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        {ROUTE_TABLE_HEADERS.map((header) => (
                          <TableHead
                            key={header}
                            className={header === "Actions" ? "text-right" : ""}
                          >
                            {header}
                          </TableHead>
                        ))}
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {combinedRoutes.map((combinedRoute, index) => (
                        <TableRow key={`${combinedRoute.type}-${index}`}>
                          <TableCell className="font-medium">
                            {combinedRoute.route.name ||
                              `${combinedRoute.type.toUpperCase()} Route ${index + 1}`}
                          </TableCell>
                          <TableCell>
                            <Badge
                              variant={combinedRoute.type === "http" ? "default" : "secondary"}
                              className={
                                combinedRoute.type === "http"
                                  ? ROUTE_TYPE_CONFIGS.http.color
                                  : ROUTE_TYPE_CONFIGS.tcp.color
                              }
                            >
                              {combinedRoute.type === "http" ? (
                                <Globe className="mr-1 h-3 w-3" />
                              ) : (
                                <Server className="mr-1 h-3 w-3" />
                              )}
                              {ROUTE_TYPE_CONFIGS[combinedRoute.type].label}
                            </Badge>
                          </TableCell>
                          <TableCell>
                            <Badge variant="outline">
                              {combinedRoute.listener.name || "unnamed"}
                            </Badge>
                          </TableCell>
                          <TableCell>
                            {combinedRoute.route.hostnames &&
                            combinedRoute.route.hostnames.length > 0 ? (
                              <div className="flex flex-wrap gap-1">
                                {combinedRoute.route.hostnames.map((hostname, i) => (
                                  <Badge key={i} variant="secondary" className="text-xs">
                                    {hostname}
                                  </Badge>
                                ))}
                              </div>
                            ) : (
                              <span className="text-muted-foreground">*</span>
                            )}
                          </TableCell>
                          <TableCell>
                            {combinedRoute.type === "http" ? (
                              (combinedRoute.route as RouteType).matches &&
                              (combinedRoute.route as RouteType).matches.length > 0 ? (
                                (combinedRoute.route as RouteType).matches.map((match, i) => (
                                  <Badge key={i} variant="outline" className="mr-1">
                                    {getPathDisplayString(match)}
                                  </Badge>
                                ))
                              ) : (
                                <span className="text-muted-foreground text-sm">*</span>
                              )
                            ) : (
                              <span className="text-muted-foreground text-sm">N/A</span>
                            )}
                          </TableCell>
                          <TableCell>
                            <Badge variant="outline">
                              {combinedRoute.route.backends?.length || 0} backend
                              {(combinedRoute.route.backends?.length || 0) !== 1 ? "s" : ""}
                            </Badge>
                          </TableCell>
                          <TableCell className="text-right">
                            <div className="flex justify-end space-x-2">
                              <Button
                                variant="ghost"
                                size="icon"
                                onClick={() => {
                                  if (combinedRoute.type === "http") {
                                    const httpRouteContext: RouteWithContext = {
                                      route: combinedRoute.route as RouteType,
                                      listener: combinedRoute.listener,
                                      bind: combinedRoute.bind,
                                      routeIndex: combinedRoute.routeIndex,
                                    };
                                    onEditRoute(httpRouteContext);
                                  } else {
                                    const tcpRouteContext: TcpRouteWithContext = {
                                      route: combinedRoute.route as TcpRoute,
                                      listener: combinedRoute.listener,
                                      bind: combinedRoute.bind,
                                      routeIndex: combinedRoute.routeIndex,
                                    };
                                    onEditTcpRoute(tcpRouteContext);
                                  }
                                }}
                                disabled={xds}
                                className={xds ? "opacity-50 cursor-not-allowed" : undefined}
                              >
                                <Edit className="h-4 w-4" />
                              </Button>
                              <Button
                                variant="ghost"
                                size="icon"
                                onClick={() => {
                                  if (combinedRoute.type === "http") {
                                    const httpRouteContext: RouteWithContext = {
                                      route: combinedRoute.route as RouteType,
                                      listener: combinedRoute.listener,
                                      bind: combinedRoute.bind,
                                      routeIndex: combinedRoute.routeIndex,
                                    };
                                    onDeleteRoute(httpRouteContext);
                                  } else {
                                    const tcpRouteContext: TcpRouteWithContext = {
                                      route: combinedRoute.route as TcpRoute,
                                      listener: combinedRoute.listener,
                                      bind: combinedRoute.bind,
                                      routeIndex: combinedRoute.routeIndex,
                                    };
                                    onDeleteTcpRoute(tcpRouteContext);
                                  }
                                }}
                                disabled={xds}
                                className={`text-destructive hover:text-destructive ${
                                  xds ? "opacity-50 cursor-not-allowed" : ""
                                }`}
                              >
                                <Trash2 className="h-4 w-4" />
                              </Button>
                            </div>
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </CardContent>
              </CollapsibleContent>
            </Collapsible>
          </Card>
        );
      })}
    </div>
  );
};

interface AddRouteDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  routeForm: typeof DEFAULT_HTTP_ROUTE_FORM;
  setRouteForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_HTTP_ROUTE_FORM>>;
  tcpRouteForm: typeof DEFAULT_TCP_ROUTE_FORM;
  setTcpRouteForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_TCP_ROUTE_FORM>>;
  selectedListener: { listener: Listener; bind: Bind } | null;
  setSelectedListener: React.Dispatch<
    React.SetStateAction<{ listener: Listener; bind: Bind } | null>
  >;
  availableListeners: { listener: Listener; bind: Bind }[];
  onAddRoute: () => void;
  onCancel: () => void;
  isSubmitting: boolean;
}

export const AddRouteDialog: React.FC<AddRouteDialogProps> = ({
  open,
  onOpenChange,
  routeForm,
  setRouteForm,
  tcpRouteForm,
  setTcpRouteForm,
  selectedListener,
  setSelectedListener,
  availableListeners,
  onAddRoute,
  onCancel,
  isSubmitting,
}) => {
  const getProtocolString = (protocol: unknown): ListenerProtocol | "HTTP" => {
    if (typeof protocol === "string") {
      return protocol as ListenerProtocol;
    }
    if (protocol && typeof protocol === "object") {
      const keys = Object.keys(protocol as Record<string, unknown>);
      if (keys.length > 0) {
        return keys[0] as ListenerProtocol;
      }
    }
    return "HTTP";
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Add Route</DialogTitle>
          <DialogDescription>
            Create a new HTTP or TCP route with matching conditions and routing rules.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {/* Route Type Selection - Auto-determined by listener protocol */}
          {selectedListener && (
            <div className="space-y-2">
              <Label>Route Type</Label>
              <div className="p-3 bg-muted rounded-md">
                <div className="flex items-center space-x-2">
                  {isTcpListener(selectedListener.listener) ? (
                    <>
                      <Server className="h-4 w-4 text-primary" />
                      <span className="font-medium">TCP Route</span>
                    </>
                  ) : (
                    <>
                      <Globe className="h-4 w-4 text-green-500" />
                      <span className="font-medium">HTTP Route</span>
                    </>
                  )}
                </div>
                <p className="text-xs text-muted-foreground mt-1">
                  Route type automatically determined by listener protocol:{" "}
                  {getProtocolString(selectedListener.listener.protocol)}
                </p>
              </div>
            </div>
          )}

          {/* Common fields for both HTTP and TCP */}
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="route-name">Route Name (optional)</Label>
              <Input
                id="route-name"
                value={
                  selectedListener && isTcpListener(selectedListener.listener)
                    ? tcpRouteForm.name
                    : routeForm.name
                }
                onChange={(e) => {
                  if (selectedListener && isTcpListener(selectedListener.listener)) {
                    setTcpRouteForm((prev) => ({ ...prev, name: e.target.value }));
                  } else {
                    setRouteForm((prev) => ({ ...prev, name: e.target.value }));
                  }
                }}
                placeholder="my-route"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="rule-name">Rule Name (optional)</Label>
              <Input
                id="rule-name"
                value={
                  selectedListener && isTcpListener(selectedListener.listener)
                    ? tcpRouteForm.ruleName
                    : routeForm.ruleName
                }
                onChange={(e) => {
                  if (selectedListener && isTcpListener(selectedListener.listener)) {
                    setTcpRouteForm((prev) => ({ ...prev, ruleName: e.target.value }));
                  } else {
                    setRouteForm((prev) => ({ ...prev, ruleName: e.target.value }));
                  }
                }}
                placeholder="rule-1"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="listener-select">Target Listener *</Label>
            {!selectedListener && (
              <p className="text-sm text-muted-foreground">
                Please select a target listener for your route:
              </p>
            )}
            <div className="grid gap-2">
              {availableListeners.map((item) => {
                const isSelected =
                  selectedListener &&
                  (selectedListener.listener.name || "unnamed") ===
                    (item.listener.name || "unnamed") &&
                  selectedListener.bind.port === item.bind.port;
                return (
                  <Button
                    key={`${item.bind.port}-${item.listener.name || "unnamed"}`}
                    variant={isSelected ? "default" : "outline"}
                    onClick={() => {
                      setSelectedListener(item);
                    }}
                    className="justify-start"
                  >
                    <Network className="mr-2 h-4 w-4" />
                    {item.listener.name || `Unnamed`} (Port {item.bind.port}) -{" "}
                    {getProtocolString(item.listener.protocol)}
                  </Button>
                );
              })}
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="hostnames">Hostnames (comma-separated)</Label>
            <Input
              id="hostnames"
              value={
                selectedListener && isTcpListener(selectedListener.listener)
                  ? tcpRouteForm.hostnames
                  : routeForm.hostnames
              }
              onChange={(e) => {
                if (selectedListener && isTcpListener(selectedListener.listener)) {
                  setTcpRouteForm((prev) => ({ ...prev, hostnames: e.target.value }));
                } else {
                  setRouteForm((prev) => ({ ...prev, hostnames: e.target.value }));
                }
              }}
              placeholder="example.com, *.example.com"
            />
            <p className="text-xs text-muted-foreground">Leave empty to match all hostnames</p>
          </div>

          {/* HTTP-specific fields */}
          {selectedListener && !isTcpListener(selectedListener.listener) && (
            <>
              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <Label htmlFor="path-type">Path Match Type</Label>
                  <Select
                    value={routeForm.pathType}
                    onValueChange={(value) =>
                      setRouteForm((prev) => ({
                        ...prev,
                        pathType: value as keyof typeof PATH_MATCH_TYPES,
                      }))
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {Object.entries(PATH_MATCH_TYPES).map(([key, label]) => (
                        <SelectItem key={key} value={key}>
                          {label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="path-value">{PATH_MATCH_TYPES[routeForm.pathType]}</Label>
                  <Input
                    id="path-value"
                    value={routeForm.pathPrefix}
                    onChange={(e) =>
                      setRouteForm((prev) => ({ ...prev, pathPrefix: e.target.value }))
                    }
                    placeholder={routeForm.pathType === "regex" ? "^/api/.*" : "/"}
                  />
                </div>
              </div>

              <div className="space-y-2">
                <Label htmlFor="headers">Headers (optional)</Label>
                <Input
                  id="headers"
                  value={routeForm.headers}
                  onChange={(e) => setRouteForm((prev) => ({ ...prev, headers: e.target.value }))}
                  placeholder="Authorization: Bearer, Content-Type: application/json"
                />
                <p className="text-xs text-muted-foreground">Format: name:value, name:value</p>
              </div>

              <div className="grid grid-cols-2 gap-4">
                <div className="space-y-2">
                  <Label htmlFor="methods">HTTP Methods (optional)</Label>
                  <Input
                    id="methods"
                    value={routeForm.methods}
                    onChange={(e) => setRouteForm((prev) => ({ ...prev, methods: e.target.value }))}
                    placeholder="GET, POST, PUT"
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="query-params">Query Parameters (optional)</Label>
                  <Input
                    id="query-params"
                    value={routeForm.queryParams}
                    onChange={(e) =>
                      setRouteForm((prev) => ({ ...prev, queryParams: e.target.value }))
                    }
                    placeholder="version=v1, type=json"
                  />
                </div>
              </div>
            </>
          )}

          {/* TCP-specific fields */}
          {selectedListener && isTcpListener(selectedListener.listener) && (
            <div className="p-4 bg-muted/50 rounded-lg">
              <div className="flex items-center space-x-2 mb-2">
                <Server className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">TCP Route Configuration</span>
              </div>
              <p className="text-sm text-muted-foreground">
                TCP routes are simpler than HTTP routes. They only support hostname-based routing
                and don&apos;t have path matching, headers, or query parameters.
              </p>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={onAddRoute} disabled={isSubmitting || !selectedListener}>
            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Add{" "}
            {selectedListener
              ? isTcpListener(selectedListener.listener)
                ? "TCP"
                : "HTTP"
              : "Route"}{" "}
            Route
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

interface EditRouteDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  routeForm: typeof DEFAULT_HTTP_ROUTE_FORM;
  setRouteForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_HTTP_ROUTE_FORM>>;
  editingRoute: RouteWithContext | null;
  onEditRoute: () => void;
  onCancel: () => void;
  isSubmitting: boolean;
  xds: boolean;
}

export const EditRouteDialog: React.FC<EditRouteDialogProps> = ({
  open,
  onOpenChange,
  routeForm,
  setRouteForm,
  editingRoute,
  onEditRoute,
  onCancel,
  isSubmitting,
  xds,
}) => {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Edit HTTP Route</DialogTitle>
          <DialogDescription>
            Update the route configuration and matching conditions.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="edit-route-name">Route Name (optional)</Label>
              <Input
                id="edit-route-name"
                value={routeForm.name}
                onChange={(e) => setRouteForm((prev) => ({ ...prev, name: e.target.value }))}
                placeholder="my-route"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-rule-name">Rule Name (optional)</Label>
              <Input
                id="edit-rule-name"
                value={routeForm.ruleName}
                onChange={(e) => setRouteForm((prev) => ({ ...prev, ruleName: e.target.value }))}
                placeholder="rule-1"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label>Current Listener</Label>
            <div className="p-3 bg-muted rounded-md">
              <div className="flex items-center space-x-2">
                <Network className="h-4 w-4" />
                <span className="font-medium">
                  {editingRoute?.listener.name || "Unnamed"} (Port {editingRoute?.bind.port})
                </span>
              </div>
              <p className="text-xs text-muted-foreground mt-1">
                Listener cannot be changed when editing. Delete and recreate the route to move it.
              </p>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="edit-hostnames">Hostnames (comma-separated)</Label>
            <Input
              id="edit-hostnames"
              value={routeForm.hostnames}
              onChange={(e) => setRouteForm((prev) => ({ ...prev, hostnames: e.target.value }))}
              placeholder="example.com, *.example.com"
            />
            <p className="text-xs text-muted-foreground">Leave empty to match all hostnames</p>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="edit-path-type">Path Match Type</Label>
              <Select
                value={routeForm.pathType}
                onValueChange={(value) =>
                  setRouteForm((prev) => ({
                    ...prev,
                    pathType: value as keyof typeof PATH_MATCH_TYPES,
                  }))
                }
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {Object.entries(PATH_MATCH_TYPES).map(([key, label]) => (
                    <SelectItem key={key} value={key}>
                      {label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-path-value">{PATH_MATCH_TYPES[routeForm.pathType]}</Label>
              <Input
                id="edit-path-value"
                value={routeForm.pathPrefix}
                onChange={(e) => setRouteForm((prev) => ({ ...prev, pathPrefix: e.target.value }))}
                placeholder={routeForm.pathType === "regex" ? "^/api/.*" : "/"}
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="edit-headers">Headers (optional)</Label>
            <Input
              id="edit-headers"
              value={routeForm.headers}
              onChange={(e) => setRouteForm((prev) => ({ ...prev, headers: e.target.value }))}
              placeholder="Authorization: Bearer, Content-Type: application/json"
            />
            <p className="text-xs text-muted-foreground">Format: name:value, name:value</p>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="edit-methods">HTTP Methods (optional)</Label>
              <Input
                id="edit-methods"
                value={routeForm.methods}
                onChange={(e) => setRouteForm((prev) => ({ ...prev, methods: e.target.value }))}
                placeholder="GET, POST, PUT"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-query-params">Query Parameters (optional)</Label>
              <Input
                id="edit-query-params"
                value={routeForm.queryParams}
                onChange={(e) => setRouteForm((prev) => ({ ...prev, queryParams: e.target.value }))}
                placeholder="version=v1, type=json"
              />
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button
            onClick={onEditRoute}
            disabled={isSubmitting || xds}
            className={xds ? "opacity-50 cursor-not-allowed" : undefined}
          >
            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Update Route
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

interface EditTcpRouteDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  tcpRouteForm: typeof DEFAULT_TCP_ROUTE_FORM;
  setTcpRouteForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_TCP_ROUTE_FORM>>;
  editingTcpRoute: TcpRouteWithContext | null;
  onEditTcpRoute: () => void;
  onCancel: () => void;
  isSubmitting: boolean;
  xds: boolean;
}

export const EditTcpRouteDialog: React.FC<EditTcpRouteDialogProps> = ({
  open,
  onOpenChange,
  tcpRouteForm,
  setTcpRouteForm,
  editingTcpRoute,
  onEditTcpRoute,
  onCancel,
  isSubmitting,
  xds,
}) => {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Edit TCP Route</DialogTitle>
          <DialogDescription>Update the TCP route configuration.</DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label htmlFor="edit-tcp-route-name">Route Name (optional)</Label>
              <Input
                id="edit-tcp-route-name"
                value={tcpRouteForm.name}
                onChange={(e) => setTcpRouteForm((prev) => ({ ...prev, name: e.target.value }))}
                placeholder="my-tcp-route"
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="edit-tcp-rule-name">Rule Name (optional)</Label>
              <Input
                id="edit-tcp-rule-name"
                value={tcpRouteForm.ruleName}
                onChange={(e) => setTcpRouteForm((prev) => ({ ...prev, ruleName: e.target.value }))}
                placeholder="tcp-rule-1"
              />
            </div>
          </div>

          <div className="space-y-2">
            <Label>Current Listener</Label>
            <div className="p-3 bg-muted rounded-md">
              <div className="flex items-center space-x-2">
                <Network className="h-4 w-4" />
                <span className="font-medium">
                  {editingTcpRoute?.listener.name || "Unnamed"} (Port {editingTcpRoute?.bind.port})
                </span>
              </div>
              <p className="text-xs text-muted-foreground mt-1">
                Listener cannot be changed when editing. Delete and recreate the route to move it.
              </p>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="edit-tcp-hostnames">Hostnames (comma-separated)</Label>
            <Input
              id="edit-tcp-hostnames"
              value={tcpRouteForm.hostnames}
              onChange={(e) => setTcpRouteForm((prev) => ({ ...prev, hostnames: e.target.value }))}
              placeholder="example.com, *.example.com"
            />
            <p className="text-xs text-muted-foreground">Leave empty to match all hostnames</p>
          </div>

          <div className="p-4 bg-muted/50 rounded-lg">
            <div className="flex items-center space-x-2 mb-2">
              <Server className="h-4 w-4 text-muted-foreground" />
              <span className="text-sm font-medium">TCP Route Configuration</span>
            </div>
            <p className="text-sm text-muted-foreground">
              TCP routes are simpler than HTTP routes. They only support hostname-based routing and
              don&apos;t have path matching, headers, or query parameters.
            </p>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button
            onClick={onEditTcpRoute}
            disabled={isSubmitting || xds}
            className={xds ? "opacity-50 cursor-not-allowed" : undefined}
          >
            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            Update TCP Route
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};
