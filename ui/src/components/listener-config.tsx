"use client";

import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Listener, ListenerProtocol, Bind } from "@/lib/types";
import {
  Trash2,
  Lock,
  Settings2,
  MoreVertical,
  Loader2,
  Plus,
  Network,
  ChevronDown,
  ChevronRight,
  ExternalLink,
} from "lucide-react";
import Link from "next/link";
import {
  fetchBinds,
  createBind,
  deleteBind,
  addListenerToBind,
  removeListenerFromBind,
} from "@/lib/api";
import { Button } from "@/components/ui/button";
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
import { Badge } from "@/components/ui/badge";
import { JWTConfigForm, TLSConfigForm, RBACConfigForm } from "@/components/forms";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useServer } from "@/lib/server-context";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { useXdsMode } from "@/hooks/use-xds-mode";

interface ListenerConfigProps {
  isAddingListener?: boolean;
  setIsAddingListener?: (isAdding: boolean) => void;
}

interface NewBindState {
  port: string;
}

interface NewListenerState {
  name: string;
  hostname: string;
  protocol: ListenerProtocol;
  targetBindPort: number | null;
}

interface ConfigDialogState {
  type: "jwt" | "tls" | "rbac" | null;
  isOpen: boolean;
  listener: Listener | null;
  bindPort: number;
  listenerIndex: number;
}

interface DeleteDialogState {
  type: "bind" | "listener";
  isOpen: boolean;
  bindPort?: number;
  listenerName?: string;
  listenerIndex?: number;
}

interface DeleteConfigDialogState {
  isOpen: boolean;
  bindPort: number;
  listenerIndex: number;
  configType: "jwt" | "tls" | "rbac" | null;
}

interface BindWithBackendsAndRoutes extends Bind {
  listeners: ListenerWithBackendsAndRoutes[];
}

interface ListenerWithBackendsAndRoutes extends Listener {
  backendCount?: number;
}

const getListenerCounts = (listener: Listener): { backendCount: number } => {
  let backendCount = 0;

  const listenerName = listener.name || "unnamed";

  // Count backends across all routes
  if (listener.routes && listener.routes.length > 0) {
    listener.routes.forEach((route) => {
      if (route.backends && route.backends.length > 0) {
        backendCount += route.backends.length;
      }
    });
  }

  console.log(`Listener ${listenerName}: ${backendCount} backends`);
  return { backendCount };
};

export function ListenerConfig({
  isAddingListener = false,
  setIsAddingListener = () => {},
}: ListenerConfigProps) {
  const { refreshListeners } = useServer();
  const xds = useXdsMode();
  const [binds, setBinds] = useState<BindWithBackendsAndRoutes[]>([]);
  const [expandedBinds, setExpandedBinds] = useState<Set<number>>(new Set());
  const [isLoading, setIsLoading] = useState(true);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isAddingBind, setIsAddingBind] = useState(false);

  const [configDialog, setConfigDialog] = useState<ConfigDialogState>({
    type: null,
    isOpen: false,
    listener: null,
    bindPort: 0,
    listenerIndex: -1,
  });

  const [newBind, setNewBind] = useState<NewBindState>({
    port: "8080",
  });

  const [newListener, setNewListener] = useState<NewListenerState>({
    name: "",
    hostname: "localhost",
    protocol: ListenerProtocol.HTTP,
    targetBindPort: null,
  });

  const [deleteDialog, setDeleteDialog] = useState<DeleteDialogState>({
    type: "bind",
    isOpen: false,
  });

  const [, setDeleteConfigDialog] = useState<DeleteConfigDialogState>({
    isOpen: false,
    bindPort: 0,
    listenerIndex: -1,
    configType: null,
  });

  // Fetch binds and their listener backend/route counts
  const loadBinds = useCallback(async () => {
    setIsLoading(true);
    try {
      const fetchedBinds = await fetchBinds();

      // Count backends directly from listener structure
      const bindsWithCounts = fetchedBinds.map((bind) => {
        const listenersWithCounts = bind.listeners.map((listener) => {
          const { backendCount } = getListenerCounts(listener);
          return {
            ...listener,
            backendCount,
          };
        });

        return {
          ...bind,
          listeners: listenersWithCounts,
        };
      });

      setBinds(bindsWithCounts);

      // Auto-expand binds that have listeners
      const newExpandedBinds = new Set<number>();
      bindsWithCounts.forEach((bind) => {
        if (bind.listeners.length > 0) {
          newExpandedBinds.add(bind.port);
        }
      });
      setExpandedBinds(newExpandedBinds);
    } catch (err) {
      console.error("Error loading binds:", err);
      toast.error(err instanceof Error ? err.message : "Failed to load binds");
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadBinds();
  }, [loadBinds]);

  const handleAddBind = async () => {
    setIsSubmitting(true);
    try {
      const port = parseInt(newBind.port, 10);
      if (isNaN(port) || port <= 0 || port > 65535) {
        throw new Error("Port must be a valid number between 1 and 65535");
      }

      await createBind(port);
      await loadBinds();
      await refreshListeners();

      setNewBind({ port: "8080" });
      setIsAddingBind(false);
      toast.success(`Bind created for port ${port}`);
    } catch (err) {
      console.error("Error adding bind:", err);
      toast.error(err instanceof Error ? err.message : "Failed to add bind");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteBind = async (port: number) => {
    setIsSubmitting(true);
    try {
      await deleteBind(port);
      await loadBinds();
      await refreshListeners();

      setDeleteDialog({ type: "bind", isOpen: false });
      toast.success(`Bind for port ${port} deleted`);
    } catch (err) {
      console.error("Error deleting bind:", err);
      toast.error(err instanceof Error ? err.message : "Failed to delete bind");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleAddListener = async () => {
    setIsSubmitting(true);
    try {
      if (!newListener.targetBindPort) {
        throw new Error("Please select a bind to add the listener to");
      }

      const listenerToAdd: Listener = {
        name: newListener.name || `listener-${Date.now()}`,
        hostname: newListener.hostname,
        protocol: newListener.protocol,
        ...(newListener.protocol === ListenerProtocol.TCP ||
        newListener.protocol === ListenerProtocol.TLS
          ? { tcpRoutes: [] }
          : { routes: [] }),
      };

      await addListenerToBind(listenerToAdd, newListener.targetBindPort);
      await loadBinds();
      await refreshListeners();

      // Expand the bind we added to
      setExpandedBinds((prev) => new Set([...prev, newListener.targetBindPort!]));

      setNewListener({
        name: "",
        hostname: "localhost",
        protocol: ListenerProtocol.HTTP,
        targetBindPort: null,
      });
      setIsAddingListener(false);
      toast.success("Listener added successfully");
    } catch (err) {
      console.error("Error adding listener:", err);
      toast.error(err instanceof Error ? err.message : "Failed to add listener");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteListener = async (listenerName: string) => {
    setIsSubmitting(true);
    try {
      await removeListenerFromBind(listenerName);
      await loadBinds();
      await refreshListeners();

      setDeleteDialog({ type: "listener", isOpen: false });
      toast.success("Listener deleted successfully");
    } catch (err) {
      console.error("Error deleting listener:", err);
      toast.error(err instanceof Error ? err.message : "Failed to delete listener");
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleAddListenerToBind = (bindPort: number) => {
    setNewListener({
      name: "",
      hostname: "localhost",
      protocol: ListenerProtocol.HTTP,
      targetBindPort: bindPort,
    });
    setIsAddingListener(true);
  };

  const handleUpdateListener = async (updatedListener: Listener, bindPort: number) => {
    setIsSubmitting(true);
    try {
      await addListenerToBind(updatedListener, bindPort);
      await loadBinds();
      await refreshListeners();

      setConfigDialog({
        type: null,
        isOpen: false,
        listener: null,
        bindPort: 0,
        listenerIndex: -1,
      });

      toast.success("Listener updated successfully");
    } catch (err) {
      console.error("Error updating listener:", err);
      toast.error(err instanceof Error ? err.message : "Failed to update listener");
    } finally {
      setIsSubmitting(false);
    }
  };

  const toggleBindExpansion = (port: number) => {
    setExpandedBinds((prev) => {
      const newSet = new Set(prev);
      if (newSet.has(port)) {
        newSet.delete(port);
      } else {
        newSet.add(port);
      }
      return newSet;
    });
  };

  const getDisplayEndpoint = (listener: ListenerWithBackendsAndRoutes, port: number) => {
    const listenerProtocol = listener.protocol || ListenerProtocol.HTTP;
    const protocol = listenerProtocol === ListenerProtocol.HTTPS ? "https" : "http";
    const hostname = listener.hostname || "localhost";
    return `${protocol}://${hostname}:${port}`;
  };

  const hasTLS = (listener: Listener) => {
    return !!listener.tls;
  };

  const getProtocolString = (protocol: unknown): ListenerProtocol => {
    if (typeof protocol === "string") {
      return protocol as ListenerProtocol;
    }
    if (protocol && typeof protocol === "object") {
      const keys = Object.keys(protocol as Record<string, unknown>);
      if (keys.length > 0) {
        return keys[0] as ListenerProtocol;
      }
    }
    return ListenerProtocol.HTTP;
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8">
        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary"></div>
        <span className="ml-2">Loading binds and listeners...</span>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex gap-2">
        <Button
          onClick={() => setIsAddingBind(true)}
          variant="outline"
          disabled={xds}
          className={xds ? "opacity-50 cursor-not-allowed" : undefined}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Bind
        </Button>
      </div>

      {binds.length === 0 ? (
        <div className="text-center py-12 border rounded-md bg-muted/20">
          <Network className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
          <p className="text-muted-foreground">
            No port binds configured. Add a bind to get started.
          </p>
        </div>
      ) : (
        <div className="space-y-4">
          {binds.map((bind) => (
            <Card key={bind.port}>
              <Collapsible
                open={expandedBinds.has(bind.port)}
                onOpenChange={() => toggleBindExpansion(bind.port)}
              >
                <CollapsibleTrigger asChild>
                  <CardHeader className="hover:bg-muted/50 cursor-pointer">
                    <div className="flex items-center justify-between">
                      <div className="flex items-center space-x-4">
                        {expandedBinds.has(bind.port) ? (
                          <ChevronDown className="h-4 w-4" />
                        ) : (
                          <ChevronRight className="h-4 w-4" />
                        )}
                        <div className="flex items-center space-x-2">
                          <Network className="h-5 w-5 text-primary" />
                          <div>
                            <h3 className="text-lg font-semibold">Port {bind.port}</h3>
                            <p className="text-sm text-muted-foreground">
                              {bind.listeners.length} listener
                              {bind.listeners.length !== 1 ? "s" : ""}
                            </p>
                          </div>
                        </div>
                      </div>
                      <div className="flex items-center space-x-2">
                        <Badge>{bind.port}</Badge>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={(e) => {
                            e.stopPropagation();
                            setDeleteDialog({
                              type: "bind",
                              isOpen: true,
                              bindPort: bind.port,
                            });
                          }}
                          disabled={xds}
                          className={`text-destructive hover:text-destructive ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </div>
                  </CardHeader>
                </CollapsibleTrigger>

                <CollapsibleContent>
                  <CardContent className="pt-0">
                    <div className="mb-4 flex justify-between items-center">
                      <h4 className="text-sm font-medium text-muted-foreground">
                        Listeners on Port {bind.port}
                      </h4>
                      <Button
                        size="sm"
                        onClick={() => handleAddListenerToBind(bind.port)}
                        disabled={xds}
                        className={`h-8 ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
                      >
                        <Plus className="mr-2 h-3 w-3" />
                        Add Listener
                      </Button>
                    </div>
                    {bind.listeners.length === 0 ? (
                      <div className="text-center py-8 text-muted-foreground">
                        No listeners in this bind. Add a listener to get started.
                      </div>
                    ) : (
                      <div className="border rounded-md overflow-hidden">
                        <Table>
                          <TableHeader>
                            <TableRow className="bg-muted/50">
                              <TableHead>Name</TableHead>
                              <TableHead>Protocol</TableHead>
                              <TableHead>Hostname</TableHead>
                              <TableHead>Endpoint</TableHead>
                              <TableHead>Backends</TableHead>
                              <TableHead>TLS</TableHead>
                              <TableHead>Policies</TableHead>
                              <TableHead className="text-right">Actions</TableHead>
                            </TableRow>
                          </TableHeader>
                          <TableBody>
                            {bind.listeners.map((listener, listenerIndex) => (
                              <TableRow
                                key={listener.name || listenerIndex}
                                className="hover:bg-muted/30"
                              >
                                <TableCell className="font-medium">
                                  <div className="flex items-center space-x-2">
                                    <span>{listener.name || `listener-${listenerIndex + 1}`}</span>
                                    {!listener.name && (
                                      <Badge variant="outline" className="text-xs">
                                        unnamed
                                      </Badge>
                                    )}
                                  </div>
                                </TableCell>
                                <TableCell>
                                  <Badge variant="outline">
                                    {getProtocolString(listener.protocol)}
                                  </Badge>
                                </TableCell>
                                <TableCell>{listener.hostname || "localhost"}</TableCell>
                                <TableCell className="font-mono text-sm">
                                  {getDisplayEndpoint(
                                    {
                                      ...listener,
                                      protocol: getProtocolString(listener.protocol),
                                    },
                                    bind.port
                                  )}
                                </TableCell>
                                <TableCell>
                                  <Badge variant="outline">
                                    {listener.backendCount ?? 0} backend
                                    {listener.backendCount !== 1 ? "s" : ""}
                                  </Badge>
                                </TableCell>
                                <TableCell>
                                  <div className="flex items-center space-x-2">
                                    <Badge
                                      variant={hasTLS(listener) ? "default" : "outline"}
                                      className="h-7 space-x-2"
                                    >
                                      <Lock className="h-4 w-4" />
                                      <span>TLS</span>
                                    </Badge>
                                    <DropdownMenu>
                                      <TooltipProvider>
                                        <Tooltip>
                                          <TooltipTrigger asChild>
                                            <DropdownMenuTrigger asChild>
                                              <Button
                                                variant="ghost"
                                                size="icon"
                                                className="hover:bg-primary/20"
                                              >
                                                <MoreVertical className="h-4 w-4" />
                                              </Button>
                                            </DropdownMenuTrigger>
                                          </TooltipTrigger>
                                          <TooltipContent side="top">
                                            <p className="text-xs">Manage TLS Encryption</p>
                                          </TooltipContent>
                                        </Tooltip>
                                      </TooltipProvider>
                                      <DropdownMenuContent align="end">
                                        <DropdownMenuItem
                                          onClick={() =>
                                            setConfigDialog({
                                              type: "tls",
                                              isOpen: true,
                                              listener,
                                              bindPort: bind.port,
                                              listenerIndex,
                                            })
                                          }
                                        >
                                          <Settings2 className="h-4 w-4 mr-2" />
                                          Configure
                                        </DropdownMenuItem>
                                        {hasTLS(listener) && (
                                          <DropdownMenuItem
                                            className={`text-destructive ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
                                            disabled={xds}
                                            onClick={() => {
                                              setDeleteConfigDialog({
                                                isOpen: true,
                                                bindPort: bind.port,
                                                listenerIndex,
                                                configType: "tls",
                                              });
                                            }}
                                          >
                                            <Trash2 className="h-4 w-4 mr-2" />
                                            Delete
                                          </DropdownMenuItem>
                                        )}
                                      </DropdownMenuContent>
                                    </DropdownMenu>
                                  </div>
                                </TableCell>
                                <TableCell>
                                  <div className="flex items-center space-x-2">
                                    <Link
                                      href="/policies"
                                      className="flex items-center underline space-x-1 hover:text-primary"
                                    >
                                      <span className="text-sm">View Policies</span>
                                      <ExternalLink className="h-3 w-3" />
                                    </Link>
                                  </div>
                                </TableCell>
                                <TableCell className="text-right">
                                  <Button
                                    variant="ghost"
                                    size="icon"
                                    onClick={() =>
                                      setDeleteDialog({
                                        type: "listener",
                                        isOpen: true,
                                        listenerName: listener.name || `listener-${listenerIndex}`,
                                        listenerIndex,
                                      })
                                    }
                                    disabled={xds}
                                    className={`text-destructive hover:text-destructive ${xds ? "opacity-50 cursor-not-allowed" : ""}`}
                                  >
                                    <Trash2 className="h-4 w-4" />
                                  </Button>
                                </TableCell>
                              </TableRow>
                            ))}
                          </TableBody>
                        </Table>
                      </div>
                    )}
                  </CardContent>
                </CollapsibleContent>
              </Collapsible>
            </Card>
          ))}
        </div>
      )}

      <Dialog open={isAddingBind} onOpenChange={setIsAddingBind}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add New Bind</DialogTitle>
            <DialogDescription>
              Create a new port binding. Listeners can then be added to this bind.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label htmlFor="bind-port">Port</Label>
              <Input
                id="bind-port"
                type="number"
                min="1"
                max="65535"
                value={newBind.port}
                onChange={(e) => setNewBind({ port: e.target.value })}
                placeholder="8080"
              />
              <p className="text-xs text-muted-foreground">
                The port number for the bind (1-65535).
              </p>
            </div>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setIsAddingBind(false)}
              disabled={isSubmitting}
            >
              Cancel
            </Button>
            <Button onClick={handleAddBind} disabled={isSubmitting || xds}>
              {isSubmitting ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Add Bind
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Add New Listener Dialog */}
      <Dialog open={isAddingListener} onOpenChange={setIsAddingListener}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Add New Listener</DialogTitle>
            <DialogDescription>
              Configure a new listener and assign it to a port bind.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <Label htmlFor="name">Name</Label>
              <Input
                id="name"
                value={newListener.name}
                onChange={(e) => setNewListener({ ...newListener, name: e.target.value })}
                placeholder="e.g., default"
              />
              <p className="text-xs text-muted-foreground">
                A unique name for this listener. If left empty, a default name will be generated.
              </p>
            </div>

            <div className="space-y-2">
              <Label>Target Bind</Label>
              <div className="grid grid-cols-2 gap-2">
                {binds.map((bind) => (
                  <Button
                    key={bind.port}
                    variant={newListener.targetBindPort === bind.port ? "default" : "outline"}
                    onClick={() => setNewListener({ ...newListener, targetBindPort: bind.port })}
                    className="justify-start"
                  >
                    <Network className="mr-2 h-4 w-4" />
                    Port {bind.port}
                  </Button>
                ))}
              </div>
              {binds.length === 0 && (
                <p className="text-sm text-muted-foreground">
                  No binds available. Create a bind first.
                </p>
              )}
              <p className="text-xs text-muted-foreground">
                Select the port bind to add this listener to.
              </p>
            </div>

            <div className="space-y-2">
              <Label>Protocol</Label>
              <RadioGroup
                defaultValue={ListenerProtocol.HTTP}
                value={newListener.protocol}
                onValueChange={(value) =>
                  setNewListener({ ...newListener, protocol: value as ListenerProtocol })
                }
                className="grid grid-cols-2 gap-4"
              >
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.HTTP} id="proto-http" />
                  <Label htmlFor="proto-http">HTTP</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.HTTPS} id="proto-https" />
                  <Label htmlFor="proto-https">HTTPS</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.TLS} id="proto-tls" />
                  <Label htmlFor="proto-tls">TLS</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.TCP} id="proto-tcp" />
                  <Label htmlFor="proto-tcp">TCP</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.HBONE} id="proto-hbone" />
                  <Label htmlFor="proto-hbone">HBONE</Label>
                </div>
              </RadioGroup>
            </div>

            <div className="space-y-2">
              <Label htmlFor="hostname">Hostname</Label>
              <Input
                id="hostname"
                value={newListener.hostname}
                onChange={(e) => setNewListener({ ...newListener, hostname: e.target.value })}
                placeholder="localhost"
              />
              <p className="text-xs text-muted-foreground">
                The hostname the listener will bind to. Can include wildcards.
              </p>
            </div>
          </div>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setIsAddingListener(false)}
              disabled={isSubmitting}
            >
              Cancel
            </Button>
            <Button
              onClick={handleAddListener}
              disabled={isSubmitting || !newListener.targetBindPort || xds}
            >
              {isSubmitting ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Add Listener
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* JWT Configuration Dialog */}
      <Dialog
        open={configDialog.type === "jwt" && configDialog.isOpen}
        onOpenChange={(open: boolean) =>
          !open && setConfigDialog({ ...configDialog, isOpen: false })
        }
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Configure JWT Authentication</DialogTitle>
            <DialogDescription>Set up JWT authentication for the listener.</DialogDescription>
          </DialogHeader>
          <JWTConfigForm
            listener={configDialog.listener}
            onSave={(listener) => handleUpdateListener(listener, configDialog.bindPort)}
            onCancel={() => setConfigDialog({ ...configDialog, isOpen: false })}
          />
        </DialogContent>
      </Dialog>

      {/* TLS Configuration Dialog */}
      <Dialog
        open={configDialog.type === "tls" && configDialog.isOpen}
        onOpenChange={(open: boolean) =>
          !open && setConfigDialog({ ...configDialog, isOpen: false })
        }
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Configure TLS</DialogTitle>
            <DialogDescription>Set up TLS encryption for the listener.</DialogDescription>
          </DialogHeader>
          <TLSConfigForm
            listener={configDialog.listener}
            onSave={(listener) => handleUpdateListener(listener, configDialog.bindPort)}
            onCancel={() => setConfigDialog({ ...configDialog, isOpen: false })}
          />
        </DialogContent>
      </Dialog>

      {/* RBAC Configuration Dialog */}
      <Dialog
        open={configDialog.type === "rbac" && configDialog.isOpen}
        onOpenChange={(open: boolean) =>
          !open && setConfigDialog({ ...configDialog, isOpen: false })
        }
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Configure RBAC</DialogTitle>
            <DialogDescription>Set up role-based access control policies.</DialogDescription>
          </DialogHeader>
          <RBACConfigForm
            listener={configDialog.listener}
            onSave={(listener) => handleUpdateListener(listener, configDialog.bindPort)}
            onCancel={() => setConfigDialog({ ...configDialog, isOpen: false })}
          />
        </DialogContent>
      </Dialog>

      {/* Delete Confirmation Dialog */}
      <Dialog
        open={deleteDialog.isOpen}
        onOpenChange={(open: boolean) =>
          !open && setDeleteDialog({ ...deleteDialog, isOpen: false })
        }
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete {deleteDialog.type === "bind" ? "Bind" : "Listener"}</DialogTitle>
            <DialogDescription>
              {deleteDialog.type === "bind"
                ? `Are you sure you want to delete the bind for port ${deleteDialog.bindPort}? This will also delete all listeners in this bind.`
                : `Are you sure you want to delete the listener "${deleteDialog.listenerName}"?`}{" "}
              This action cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setDeleteDialog({ ...deleteDialog, isOpen: false })}
              disabled={isSubmitting}
            >
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => {
                if (deleteDialog.type === "bind" && deleteDialog.bindPort) {
                  handleDeleteBind(deleteDialog.bindPort);
                } else if (deleteDialog.type === "listener" && deleteDialog.listenerName) {
                  handleDeleteListener(deleteDialog.listenerName);
                }
              }}
              disabled={isSubmitting || xds}
              className={xds ? "opacity-50 cursor-not-allowed" : undefined}
            >
              {isSubmitting ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
              Delete
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
