"use client";

import React from "react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Plus,
  Target,
  ChevronDown,
  ChevronRight,
  Trash2,
  Edit,
  Brain,
  Cloud,
  Server,
  Globe,
  Loader2,
  Shield,
  AlertTriangle,
} from "lucide-react";
import { Bind } from "@/lib/types";
import { BackendWithContext } from "@/lib/backend-hooks";
import {
  DEFAULT_BACKEND_FORM,
  BACKEND_TYPES,
  BACKEND_TABLE_HEADERS,
  HOST_TYPES,
  AI_MODEL_PLACEHOLDERS,
  AI_REGION_PLACEHOLDERS,
} from "@/lib/backend-constants";
import {
  getBackendType,
  getBackendName,
  getBackendTypeColor,
  getBackendDetails,
  getAvailableRoutes,
  AI_PROVIDERS,
  MCP_TARGET_TYPES,
  hasBackendPolicies,
  getBackendPolicyTypes,
  canDeleteBackend,
} from "@/lib/backend-utils";
import { useXdsMode } from "@/hooks/use-xds-mode";

const getEnvAsRecord = (env: unknown): Record<string, string> => {
  return typeof env === "object" && env !== null ? (env as Record<string, string>) : {};
};

// Icon mapping
const getBackendIcon = (type: string) => {
  switch (type) {
    case "mcp":
      return <Target className="h-4 w-4" />;
    case "ai":
      return <Brain className="h-4 w-4" />;
    case "service":
      return <Cloud className="h-4 w-4" />;
    case "host":
      return <Server className="h-4 w-4" />;
    case "dynamic":
      return <Globe className="h-4 w-4" />;
    default:
      return <Server className="h-4 w-4" />;
  }
};

interface BackendTableProps {
  backendsByBind: Map<number, BackendWithContext[]>;
  expandedBinds: Set<number>;
  setExpandedBinds: React.Dispatch<React.SetStateAction<Set<number>>>;
  onEditBackend: (backendContext: BackendWithContext) => void;
  onDeleteBackend: (backendContext: BackendWithContext) => void;
  isSubmitting: boolean;
}

export const BackendTable: React.FC<BackendTableProps> = ({
  backendsByBind,
  expandedBinds,
  setExpandedBinds,
  onEditBackend,
  onDeleteBackend,
  isSubmitting,
}) => {
  const xds = useXdsMode();
  return (
    <div className="space-y-4">
      {Array.from(backendsByBind.entries()).map(([port, backendContexts]) => {
        const typeCounts = backendContexts.reduce(
          (acc, bc) => {
            const type = getBackendType(bc.backend);
            acc[type] = (acc[type] || 0) + 1;
            return acc;
          },
          {} as Record<string, number>
        );

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
                          {Object.entries(typeCounts).map(([type, count]) => (
                            <div key={type} className="flex items-center space-x-1">
                              {getBackendIcon(type)}
                              <span>
                                {count} {type.toUpperCase()}
                              </span>
                            </div>
                          ))}
                        </div>
                      </div>
                    </div>
                    <Badge>{backendContexts.length} backends</Badge>
                  </div>
                </CardHeader>
              </CollapsibleTrigger>

              <CollapsibleContent>
                <CardContent className="pt-0">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        {BACKEND_TABLE_HEADERS.map((header) => (
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
                      {backendContexts.map((backendContext, index) => {
                        const type = getBackendType(backendContext.backend);
                        return (
                          <TableRow key={index}>
                            <TableCell className="font-medium">
                              {getBackendName(backendContext.backend)}
                            </TableCell>
                            <TableCell>
                              <Badge
                                variant="secondary"
                                className={`${getBackendTypeColor(type)} text-white`}
                              >
                                {getBackendIcon(type)}
                                <span className="ml-1 capitalize">{type}</span>
                              </Badge>
                            </TableCell>
                            <TableCell>
                              <Badge variant="outline">
                                {backendContext.listener.name || "unnamed"}
                              </Badge>
                            </TableCell>
                            <TableCell>
                              <Badge variant="outline">
                                {backendContext.route.name ||
                                  `Route ${backendContext.routeIndex + 1}`}
                              </Badge>
                            </TableCell>
                            <TableCell className="text-sm text-muted-foreground">
                              {(() => {
                                const details = getBackendDetails(backendContext.backend);
                                const hasPolicies = hasBackendPolicies(backendContext.route);
                                const policyTypes = hasPolicies
                                  ? getBackendPolicyTypes(backendContext.route)
                                  : [];

                                return (
                                  <div className="space-y-1">
                                    <div>{details.primary}</div>
                                    {details.secondary && (
                                      <div className="text-xs text-muted-foreground/80 font-mono">
                                        {details.secondary}
                                      </div>
                                    )}
                                    {hasPolicies && (
                                      <div className="flex items-center space-x-1 mt-1">
                                        <Shield className="h-3 w-3 text-primary" />
                                        <span className="text-xs text-primary font-medium">
                                          Backend Policies: {policyTypes.join(", ")}
                                        </span>
                                      </div>
                                    )}
                                  </div>
                                );
                              })()}
                            </TableCell>
                            <TableCell>
                              <Badge>{backendContext.backend.weight || 1}</Badge>
                            </TableCell>
                            <TableCell className="text-right">
                              <div className="flex justify-end space-x-2">
                                <Button
                                  variant="ghost"
                                  size="icon"
                                  onClick={() => onEditBackend(backendContext)}
                                  disabled={xds}
                                  className={xds ? "opacity-50 cursor-not-allowed" : undefined}
                                >
                                  <Edit className="h-4 w-4" />
                                </Button>
                                {(() => {
                                  // Check if deletion is allowed
                                  const totalBackendsInRoute = backendContexts.filter(
                                    (bc) =>
                                      bc.bind.port === backendContext.bind.port &&
                                      bc.listener.name === backendContext.listener.name &&
                                      bc.routeIndex === backendContext.routeIndex
                                  ).length;

                                  const deleteCheck = canDeleteBackend(
                                    backendContext.route,
                                    totalBackendsInRoute
                                  );

                                  if (!deleteCheck.canDelete) {
                                    return (
                                      <TooltipProvider>
                                        <Tooltip>
                                          <TooltipTrigger asChild>
                                            <div>
                                              <Button
                                                variant="ghost"
                                                size="icon"
                                                disabled={true}
                                                className="text-muted-foreground cursor-not-allowed"
                                              >
                                                <div className="relative">
                                                  <Trash2 className="h-4 w-4" />
                                                  <AlertTriangle className="h-2 w-2 absolute -top-0.5 -right-0.5 text-amber-500" />
                                                </div>
                                              </Button>
                                            </div>
                                          </TooltipTrigger>
                                          <TooltipContent className="max-w-sm">
                                            <p>{deleteCheck.reason}</p>
                                          </TooltipContent>
                                        </Tooltip>
                                      </TooltipProvider>
                                    );
                                  }

                                  return (
                                    <Button
                                      variant="ghost"
                                      size="icon"
                                      onClick={() => onDeleteBackend(backendContext)}
                                      className={`text-destructive hover:text-destructive ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
                                      disabled={isSubmitting || xds}
                                    >
                                      <Trash2 className="h-4 w-4" />
                                    </Button>
                                  );
                                })()}
                              </div>
                            </TableCell>
                          </TableRow>
                        );
                      })}
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

interface AddBackendDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  binds: Bind[];
  backendForm: typeof DEFAULT_BACKEND_FORM;
  setBackendForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_BACKEND_FORM>>;
  selectedBackendType: string;
  setSelectedBackendType: React.Dispatch<React.SetStateAction<string>>;
  editingBackend: BackendWithContext | null;
  onAddBackend: () => void;
  onCancel: () => void;
  isSubmitting: boolean;
  // MCP target management
  addMcpTarget: () => void;
  removeMcpTarget: (index: number) => void;
  updateMcpTarget: (index: number, field: string, value: any) => void;
  parseAndUpdateUrl: (index: number, url: string) => void;
  updateMcpStateful: (stateful: boolean) => void;
}

export const AddBackendDialog: React.FC<AddBackendDialogProps> = ({
  open,
  onOpenChange,
  binds,
  backendForm,
  setBackendForm,
  selectedBackendType,
  setSelectedBackendType,
  editingBackend,
  onAddBackend,
  onCancel,
  isSubmitting,
  addMcpTarget,
  removeMcpTarget,
  updateMcpTarget,
  parseAndUpdateUrl,
  updateMcpStateful,
}) => {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {editingBackend
              ? `Edit Backend: ${getBackendName(editingBackend.backend)}`
              : `Add ${selectedBackendType.toUpperCase()} Backend`}
          </DialogTitle>
          <DialogDescription>
            {editingBackend
              ? "Update the backend configuration."
              : "Configure a new backend for your routes."}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 py-4">
          {/* Backend Type Selection */}
          <div className="space-y-2">
            <Label>Backend Type *</Label>
            <div className="grid grid-cols-2 gap-2">
              {BACKEND_TYPES.map(({ value, label, icon }) => {
                const IconComponent = {
                  Target,
                  Brain,
                  Cloud,
                  Server,
                  Globe,
                }[icon];

                return (
                  <Button
                    key={value}
                    type="button"
                    variant={selectedBackendType === value ? "default" : "outline"}
                    onClick={() => setSelectedBackendType(value)}
                    className="justify-start"
                  >
                    <IconComponent className="mr-2 h-4 w-4" />
                    {label}
                  </Button>
                );
              })}
            </div>
          </div>

          {/* Common fields */}
          <div
            className={
              selectedBackendType === "ai" || selectedBackendType === "mcp"
                ? "space-y-4"
                : "grid grid-cols-2 gap-4"
            }
          >
            {/* Only show name input for backends that support custom names */}
            {selectedBackendType !== "mcp" && (
              <div className="space-y-2">
                <Label htmlFor="backend-name">Name *</Label>
                <Input
                  id="backend-name"
                  value={backendForm.name}
                  onChange={(e) => setBackendForm((prev) => ({ ...prev, name: e.target.value }))}
                  placeholder="Backend name"
                />
              </div>
            )}
            <div className="space-y-2">
              <Label htmlFor="backend-weight">Weight</Label>
              <Input
                id="backend-weight"
                type="number"
                min="0"
                step="1"
                value={backendForm.weight}
                onChange={(e) => setBackendForm((prev) => ({ ...prev, weight: e.target.value }))}
                placeholder="1"
              />
              <p className="text-xs text-muted-foreground">
                Weight determines load balancing priority. Higher values get more traffic.
              </p>
            </div>
          </div>

          {/* Route Selection */}
          <div className="space-y-2">
            <Label>Route *</Label>
            {editingBackend ? (
              <div className="p-3 bg-muted rounded-md">
                <p className="text-sm">
                  Port {editingBackend.bind.port} → {editingBackend.listener.name || "unnamed"} →{" "}
                  {editingBackend.route.name || `Route ${editingBackend.routeIndex + 1}`}
                </p>
                <p className="text-xs text-muted-foreground">
                  Route cannot be changed when editing
                </p>
              </div>
            ) : (
              <Select
                value={`${backendForm.selectedBindPort}-${backendForm.selectedListenerName}-${backendForm.selectedRouteIndex}`}
                onValueChange={(value) => {
                  const [bindPort, listenerName, routeIndex] = value.split("-");
                  setBackendForm((prev) => ({
                    ...prev,
                    selectedBindPort: bindPort,
                    selectedListenerName: listenerName,
                    selectedRouteIndex: routeIndex,
                  }));
                }}
              >
                <SelectTrigger>
                  <SelectValue placeholder="Select a route" />
                </SelectTrigger>
                <SelectContent>
                  {getAvailableRoutes(binds).length === 0 ? (
                    <div className="py-2 px-3 text-sm text-muted-foreground">
                      No routes available. Create a route first.
                    </div>
                  ) : (
                    getAvailableRoutes(binds).map((route) => (
                      <SelectItem
                        key={`${route.bindPort}-${route.listenerName}-${route.routeIndex}`}
                        value={`${route.bindPort}-${route.listenerName}-${route.routeIndex}`}
                      >
                        Port {route.bindPort} → {route.listenerName} → {route.routeName} (
                        {route.path})
                      </SelectItem>
                    ))
                  )}
                </SelectContent>
              </Select>
            )}
          </div>

          {/* Service Backend Configuration */}
          {selectedBackendType === "service" && (
            <ServiceBackendForm backendForm={backendForm} setBackendForm={setBackendForm} />
          )}

          {/* Host Backend Configuration */}
          {selectedBackendType === "host" && (
            <HostBackendForm backendForm={backendForm} setBackendForm={setBackendForm} />
          )}

          {/* MCP Backend Configuration */}
          {selectedBackendType === "mcp" && (
            <McpBackendForm
              backendForm={backendForm}
              addMcpTarget={addMcpTarget}
              removeMcpTarget={removeMcpTarget}
              updateMcpTarget={updateMcpTarget}
              parseAndUpdateUrl={parseAndUpdateUrl}
              updateMcpStateful={updateMcpStateful}
            />
          )}

          {/* AI Backend Configuration */}
          {selectedBackendType === "ai" && (
            <AiBackendForm backendForm={backendForm} setBackendForm={setBackendForm} />
          )}

          {/* Dynamic Backend Configuration */}
          {selectedBackendType === "dynamic" && (
            <div className="p-4 bg-muted/50 rounded-lg">
              <div className="flex items-center space-x-2 mb-2">
                <Globe className="h-4 w-4 text-muted-foreground" />
                <span className="text-sm font-medium">Dynamic Backend</span>
              </div>
              <p className="text-sm text-muted-foreground">
                Dynamic backends are automatically configured and don&apos;t require additional
                settings.
              </p>
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button onClick={onAddBackend} disabled={isSubmitting}>
            {isSubmitting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
            {editingBackend ? "Update" : "Add"} {selectedBackendType.toUpperCase()} Backend
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
};

// Service Backend Form Component
interface ServiceBackendFormProps {
  backendForm: typeof DEFAULT_BACKEND_FORM;
  setBackendForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_BACKEND_FORM>>;
}

const ServiceBackendForm: React.FC<ServiceBackendFormProps> = ({ backendForm, setBackendForm }) => (
  <div className="space-y-4">
    <div className="grid grid-cols-2 gap-4">
      <div className="space-y-2">
        <Label htmlFor="service-namespace">Namespace *</Label>
        <Input
          id="service-namespace"
          value={backendForm.serviceNamespace}
          onChange={(e) =>
            setBackendForm((prev) => ({ ...prev, serviceNamespace: e.target.value }))
          }
          placeholder="default"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="service-hostname">Hostname *</Label>
        <Input
          id="service-hostname"
          value={backendForm.serviceHostname}
          onChange={(e) => setBackendForm((prev) => ({ ...prev, serviceHostname: e.target.value }))}
          placeholder="my-service"
        />
      </div>
    </div>
    <div className="space-y-2">
      <Label htmlFor="service-port">Port *</Label>
      <Input
        id="service-port"
        type="number"
        min="0"
        max="65535"
        value={backendForm.servicePort}
        onChange={(e) => setBackendForm((prev) => ({ ...prev, servicePort: e.target.value }))}
        placeholder="80"
      />
    </div>
  </div>
);

// Host Backend Form Component
interface HostBackendFormProps {
  backendForm: typeof DEFAULT_BACKEND_FORM;
  setBackendForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_BACKEND_FORM>>;
}

const HostBackendForm: React.FC<HostBackendFormProps> = ({ backendForm, setBackendForm }) => (
  <div className="space-y-4">
    <div className="space-y-2">
      <Label>Host Type *</Label>
      <div className="flex space-x-4">
        {HOST_TYPES.map(({ value, label }) => (
          <Button
            key={value}
            type="button"
            variant={backendForm.hostType === value ? "default" : "outline"}
            onClick={() => setBackendForm((prev) => ({ ...prev, hostType: value as any }))}
          >
            {label}
          </Button>
        ))}
      </div>
    </div>

    {backendForm.hostType === "address" ? (
      <div className="space-y-2">
        <Label htmlFor="host-address">Address *</Label>
        <Input
          id="host-address"
          value={backendForm.hostAddress}
          onChange={(e) => setBackendForm((prev) => ({ ...prev, hostAddress: e.target.value }))}
          placeholder="192.168.1.100:8080"
        />
      </div>
    ) : (
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="host-hostname">Hostname *</Label>
          <Input
            id="host-hostname"
            value={backendForm.hostHostname}
            onChange={(e) => setBackendForm((prev) => ({ ...prev, hostHostname: e.target.value }))}
            placeholder="example.com"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="host-port">Port *</Label>
          <Input
            id="host-port"
            type="number"
            min="0"
            max="65535"
            value={backendForm.hostPort}
            onChange={(e) => setBackendForm((prev) => ({ ...prev, hostPort: e.target.value }))}
            placeholder="8080"
          />
        </div>
      </div>
    )}
  </div>
);

// MCP Backend Form Component
interface McpBackendFormProps {
  backendForm: typeof DEFAULT_BACKEND_FORM;
  addMcpTarget: () => void;
  removeMcpTarget: (index: number) => void;
  updateMcpTarget: (index: number, field: string, value: any) => void;
  parseAndUpdateUrl: (index: number, url: string) => void;
  updateMcpStateful: (stateful: boolean) => void;
}

const McpBackendForm: React.FC<McpBackendFormProps> = ({
  backendForm,
  addMcpTarget,
  removeMcpTarget,
  updateMcpTarget,
  parseAndUpdateUrl,
  updateMcpStateful,
}) => (
  <div className="space-y-4">
    <div className="flex items-center justify-between">
      <Label>MCP Targets</Label>
      <Button type="button" variant="outline" size="sm" onClick={addMcpTarget}>
        <Plus className="mr-1 h-3 w-3" />
        Add Target
      </Button>
    </div>

    {backendForm.mcpTargets.map((target, index) => (
      <Card key={index} className="p-4">
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h4 className="text-sm font-medium">Target {index + 1}</h4>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => removeMcpTarget(index)}
              className="text-destructive hover:text-destructive"
            >
              <Trash2 className="h-3 w-3" />
            </Button>
          </div>

          <div className="grid grid-cols-2 gap-4">
            <div className="space-y-2">
              <Label>Target Name *</Label>
              <Input
                value={target.name}
                onChange={(e) => updateMcpTarget(index, "name", e.target.value)}
                placeholder="my-target"
              />
            </div>
            <div className="space-y-2">
              <Label>Target Type *</Label>
              <Select
                value={target.type}
                onValueChange={(value) => updateMcpTarget(index, "type", value)}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {MCP_TARGET_TYPES.map(({ value, label }) => (
                    <SelectItem key={value} value={value}>
                      {label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          {(target.type === "sse" || target.type === "mcp" || target.type === "openapi") && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>URL *</Label>
                <Input
                  value={target.fullUrl}
                  onChange={(e) => parseAndUpdateUrl(index, e.target.value)}
                  placeholder="https://example.com/mcp"
                />
              </div>
            </div>
          )}

          {target.type === "stdio" && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label>Command *</Label>
                <Input
                  value={target.cmd}
                  onChange={(e) => updateMcpTarget(index, "cmd", e.target.value)}
                  placeholder="python3 my_mcp_server.py"
                />
              </div>
              {/* Arguments Section */}
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label>Arguments</Label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      const currentArgs = Array.isArray(target.args) ? target.args : [];
                      updateMcpTarget(index, "args", [...currentArgs, ""]);
                    }}
                  >
                    <Plus className="mr-1 h-3 w-3" />
                    Add Argument
                  </Button>
                </div>
                {Array.isArray(target.args) && target.args.length > 0 ? (
                  <div className="space-y-2">
                    {target.args.map((arg, argIndex) => (
                      <div key={argIndex} className="flex items-center space-x-2">
                        <Input
                          value={arg}
                          onChange={(e) => {
                            const newArgs = [...target.args];
                            newArgs[argIndex] = e.target.value;
                            updateMcpTarget(index, "args", newArgs);
                          }}
                          placeholder="--verbose"
                          className="flex-1"
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={() => {
                            const newArgs = Array.isArray(target.args)
                              ? target.args.filter((_, i: number) => i !== argIndex)
                              : [];
                            updateMcpTarget(index, "args", newArgs);
                          }}
                          className="text-destructive hover:text-destructive"
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="text-center py-4 border-2 border-dashed border-muted rounded-md">
                    <p className="text-sm text-muted-foreground">No arguments configured</p>
                  </div>
                )}
              </div>

              {/* Environment Variables Section */}
              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label>Environment Variables</Label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      const currentEnv = getEnvAsRecord(target.env);
                      updateMcpTarget(index, "env", { ...currentEnv, "": "" });
                    }}
                  >
                    <Plus className="mr-1 h-3 w-3" />
                    Add Variable
                  </Button>
                </div>
                {Object.keys(getEnvAsRecord(target.env)).length > 0 ? (
                  <div className="space-y-2">
                    {Object.entries(getEnvAsRecord(target.env)).map(([key, value], envIndex) => (
                      <div key={envIndex} className="flex items-center space-x-2">
                        <Input
                          value={key}
                          onChange={(e) => {
                            const currentEnv = getEnvAsRecord(target.env);
                            const newEnv = { ...currentEnv };
                            delete newEnv[key];
                            newEnv[e.target.value] = String(value);
                            updateMcpTarget(index, "env", newEnv);
                          }}
                          placeholder="DEBUG"
                          className="flex-1"
                        />
                        <span className="text-muted-foreground">=</span>
                        <Input
                          value={String(value)}
                          onChange={(e) => {
                            const currentEnv = getEnvAsRecord(target.env);
                            const newEnv = { ...currentEnv };
                            newEnv[key] = e.target.value;
                            updateMcpTarget(index, "env", newEnv);
                          }}
                          placeholder="true"
                          className="flex-1"
                        />
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          onClick={() => {
                            const currentEnv = getEnvAsRecord(target.env);
                            const newEnv = { ...currentEnv };
                            delete newEnv[key];
                            updateMcpTarget(index, "env", newEnv);
                          }}
                          className="text-destructive hover:text-destructive"
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      </div>
                    ))}
                  </div>
                ) : (
                  <div className="text-center py-4 border-2 border-dashed border-muted rounded-md">
                    <p className="text-sm text-muted-foreground">
                      No environment variables configured
                    </p>
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      </Card>
    ))}

    {backendForm.mcpTargets.length === 0 && (
      <div className="text-center py-8 border-2 border-dashed border-muted rounded-md">
        <Target className="mx-auto h-8 w-8 text-muted-foreground mb-2" />
        <p className="text-sm text-muted-foreground">No targets configured</p>
        <p className="text-xs text-muted-foreground">
          Add at least one target to create an MCP backend
        </p>
      </div>
    )}

    <div className="flex items-center space-x-2">
      <input
        type="checkbox"
        id="mcp-stateful"
        checked={!!backendForm.mcpStateful}
        onChange={(e) => updateMcpStateful(e.target.checked)}
        className="form-checkbox h-4 w-4"
      />
      <Label htmlFor="mcp-stateful" className="cursor-pointer">
        Enable stateful mode
      </Label>
    </div>
  </div>
);

// AI Backend Form Component
interface AiBackendFormProps {
  backendForm: typeof DEFAULT_BACKEND_FORM;
  setBackendForm: React.Dispatch<React.SetStateAction<typeof DEFAULT_BACKEND_FORM>>;
}

const AiBackendForm: React.FC<AiBackendFormProps> = ({ backendForm, setBackendForm }) => (
  <div className="space-y-4">
    <div className="space-y-2">
      <Label>AI Provider *</Label>
      <div className="grid grid-cols-3 gap-2">
        {AI_PROVIDERS.map(({ value, label }) => (
          <Button
            key={value}
            type="button"
            variant={backendForm.aiProvider === value ? "default" : "outline"}
            onClick={() => setBackendForm((prev) => ({ ...prev, aiProvider: value as any }))}
            className="text-sm"
          >
            {label}
          </Button>
        ))}
      </div>
    </div>

    <div className="grid grid-cols-2 gap-4">
      <div className="space-y-2">
        <Label htmlFor="ai-model">
          Model {backendForm.aiProvider === "bedrock" ? "*" : "(optional)"}
        </Label>
        <Input
          id="ai-model"
          value={backendForm.aiModel}
          onChange={(e) => setBackendForm((prev) => ({ ...prev, aiModel: e.target.value }))}
          placeholder={AI_MODEL_PLACEHOLDERS[backendForm.aiProvider]}
        />
      </div>

      {(backendForm.aiProvider === "vertex" || backendForm.aiProvider === "bedrock") && (
        <div className="space-y-2">
          <Label htmlFor="ai-region">
            Region {backendForm.aiProvider === "bedrock" ? "*" : "(optional)"}
          </Label>
          <Input
            id="ai-region"
            value={backendForm.aiRegion}
            onChange={(e) => setBackendForm((prev) => ({ ...prev, aiRegion: e.target.value }))}
            placeholder={AI_REGION_PLACEHOLDERS[backendForm.aiProvider]}
          />
        </div>
      )}
    </div>

    {backendForm.aiProvider === "vertex" && (
      <div className="space-y-2">
        <Label htmlFor="ai-project-id">Project ID *</Label>
        <Input
          id="ai-project-id"
          value={backendForm.aiProjectId}
          onChange={(e) => setBackendForm((prev) => ({ ...prev, aiProjectId: e.target.value }))}
          placeholder="my-gcp-project"
        />
      </div>
    )}

    {backendForm.aiProvider === "azureOpenAI" && (
      <div className="space-y-2">
        <Label htmlFor="ai-host">Host *</Label>
        <Input
          id="ai-host"
          value={backendForm.aiHost}
          onChange={(e) => setBackendForm((prev) => ({ ...prev, aiHost: e.target.value }))}
          placeholder="my-resource-name.openai.azure.com"
        />
      </div>
    )}
    {backendForm.aiProvider === "azureOpenAI" && (
      <div className="space-y-2">
        <Label htmlFor="ai-api-version">API Version (optional)</Label>
        <Input
          id="ai-api-version"
          value={backendForm.aiApiVersion}
          onChange={(e) => setBackendForm((prev) => ({ ...prev, aiApiVersion: e.target.value }))}
          placeholder="v1, preview, 2024-10-21, etc. (defaults to v1)"
        />
      </div>
    )}

    {/* AI Host Override */}
    <div className="space-y-2">
      <Label htmlFor="ai-host-override">Host Override (optional)</Label>
      <Input
        id="ai-host-override"
        value={backendForm.aiHostOverride}
        onChange={(e) => setBackendForm((prev) => ({ ...prev, aiHostOverride: e.target.value }))}
        placeholder="api.custom-ai-provider.com:443"
      />
    </div>

    {/* AI Path Override */}
    <div className="space-y-2">
      <Label htmlFor="ai-path-override">Path Override (optional)</Label>
      <Input
        id="ai-path-override"
        value={backendForm.aiPathOverride}
        onChange={(e) => setBackendForm((prev) => ({ ...prev, aiPathOverride: e.target.value }))}
        placeholder="/v1/chat/completions"
      />
    </div>
  </div>
);
