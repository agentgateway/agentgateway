import { useCallback, useState } from "react";
import { Backend, Route, Listener, Bind } from "@/lib/types";
import { fetchConfig, updateConfig } from "@/lib/api";
import { useServer } from "@/lib/server-context";
import { toast } from "sonner";
import { DEFAULT_BACKEND_FORM, DEFAULT_MCP_TARGET } from "./backend-constants";
import {
  getBackendType,
  populateFormFromBackend,
  parseUrl,
  validateBackendForm,
  createBackendFromForm,
  getAvailableRoutes,
  canDeleteBackend,
} from "./backend-utils";

export interface BackendWithContext {
  backend: Backend;
  route: Route;
  listener: Listener;
  bind: Bind;
  backendIndex: number;
  routeIndex: number;
}

// Hook for managing backend data loading and organization
export const useBackendData = () => {
  const [binds, setBinds] = useState<Bind[]>([]);
  const [backends, setBackends] = useState<BackendWithContext[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [expandedBinds, setExpandedBinds] = useState<Set<number>>(new Set());

  const loadBackends = useCallback(async () => {
    setIsLoading(true);
    try {
      const config = await fetchConfig();
      const binds = config.binds;
      setBinds(binds);
      const backends: BackendWithContext[] = [];

      binds.forEach((bind) => {
        bind.listeners.forEach((listener) => {
          listener.routes?.forEach((route, routeIndex) => {
            route.backends?.forEach((backend, backendIndex) => {
              backends.push({
                backend,
                route,
                listener,
                bind,
                backendIndex,
                routeIndex,
              });
            });
          });
        });
      });
      setBackends(backends);

      // Auto-expand binds with backends
      const bindsWithBackends = new Set<number>();
      backends.forEach(({ bind }) => bindsWithBackends.add(bind.port));
      setExpandedBinds(bindsWithBackends);
    } catch (err) {
      console.error("Error loading backends:", err);
      toast.error("Failed to load backends");
    } finally {
      setIsLoading(false);
    }
  }, []);

  const getBackendsByBind = () => {
    const backendsByBind = new Map<number, BackendWithContext[]>();
    backends.forEach((backendContext) => {
      const port = backendContext.bind.port;
      if (!backendsByBind.has(port)) {
        backendsByBind.set(port, []);
      }
      backendsByBind.get(port)!.push(backendContext);
    });
    return backendsByBind;
  };

  return {
    binds,
    backends,
    isLoading,
    expandedBinds,
    setExpandedBinds,
    loadBackends,
    getBackendsByBind,
  };
};

// Hook for managing form state
export const useBackendFormState = () => {
  const [backendForm, setBackendForm] = useState(DEFAULT_BACKEND_FORM);
  const [selectedBackendType, setSelectedBackendType] = useState<string>("mcp");

  const resetBackendForm = (availableBinds?: Bind[]) => {
    const formWithDefaults = { ...DEFAULT_BACKEND_FORM };

    // Auto-select the first available route if any exist
    if (availableBinds && availableBinds.length > 0) {
      const availableRoutes = getAvailableRoutes(availableBinds);
      if (availableRoutes.length > 0) {
        const firstRoute = availableRoutes[0];
        formWithDefaults.selectedBindPort = firstRoute.bindPort.toString();
        formWithDefaults.selectedListenerName = firstRoute.listenerName;
        formWithDefaults.selectedRouteIndex = firstRoute.routeIndex.toString();
      }
    }

    setBackendForm(formWithDefaults);
    setSelectedBackendType("mcp");
  };

  const populateFormFromBackendContext = (backendContext: BackendWithContext) => {
    const { backend, bind, listener, routeIndex } = backendContext;
    const backendType = getBackendType(backend);
    const formData = populateFormFromBackend(backend, bind, listener, routeIndex);

    setSelectedBackendType(backendType);
    setBackendForm(formData);
  };

  // MCP target management
  const addMcpTarget = () => {
    setBackendForm((prev) => ({
      ...prev,
      mcpTargets: [...prev.mcpTargets, { ...DEFAULT_MCP_TARGET }],
    }));
  };

  const removeMcpTarget = (index: number) => {
    setBackendForm((prev) => ({
      ...prev,
      mcpTargets: prev.mcpTargets.filter((_, i) => i !== index),
    }));
  };

  const updateMcpTarget = (index: number, field: string, value: any) => {
    setBackendForm((prev) => ({
      ...prev,
      mcpTargets: prev.mcpTargets.map((target, i) =>
        i === index ? { ...target, [field]: value } : target
      ),
    }));
  };

  const updateMcpStateful = (stateful: boolean) => {
    setBackendForm((prev) => ({
      ...prev,
      mcpStateful: stateful,
    }));
  };

  const parseAndUpdateUrl = (index: number, url: string) => {
    const { host, port, path } = parseUrl(url);

    setBackendForm((prev) => ({
      ...prev,
      mcpTargets: prev.mcpTargets.map((target, i) =>
        i === index
          ? {
              ...target,
              fullUrl: url,
              host,
              port,
              path,
            }
          : target
      ),
    }));
  };

  return {
    backendForm,
    setBackendForm,
    selectedBackendType,
    setSelectedBackendType,
    resetBackendForm,
    populateFormFromBackendContext,
    addMcpTarget,
    removeMcpTarget,
    updateMcpTarget,
    parseAndUpdateUrl,
    updateMcpStateful,
  };
};

// Hook for managing dialog states
export const useBackendDialogs = () => {
  const [isAddBackendDialogOpen, setIsAddBackendDialogOpen] = useState(false);
  const [editingBackend, setEditingBackend] = useState<BackendWithContext | null>(null);

  const openAddDialog = () => {
    setIsAddBackendDialogOpen(true);
    setEditingBackend(null);
  };

  const openEditDialog = (backendContext: BackendWithContext) => {
    setEditingBackend(backendContext);
    setIsAddBackendDialogOpen(true);
  };

  const closeDialogs = () => {
    setIsAddBackendDialogOpen(false);
    setEditingBackend(null);
  };

  return {
    isAddBackendDialogOpen,
    setIsAddBackendDialogOpen,
    editingBackend,
    setEditingBackend,
    openAddDialog,
    openEditDialog,
    closeDialogs,
  };
};

// Hook for managing backend operations (CRUD)
export const useBackendOperations = () => {
  const { refreshListeners } = useServer();
  const [isSubmitting, setIsSubmitting] = useState(false);

  const addBackend = async (
    form: typeof DEFAULT_BACKEND_FORM,
    backendType: string,
    editingBackend: BackendWithContext | null,
    onSuccess: () => void
  ) => {
    if (!validateBackendForm(form, backendType, !!editingBackend)) {
      const weight = parseInt(form.weight);
      if (isNaN(weight) || weight < 0) {
        toast.error("Weight must be a positive integer");
      } else {
        toast.error("Please fill in all required fields");
      }
      return;
    }

    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Use editing backend's route info or form values
      const bindPort = editingBackend ? editingBackend.bind.port : parseInt(form.selectedBindPort);
      const routeIndex = editingBackend
        ? editingBackend.routeIndex
        : parseInt(form.selectedRouteIndex);
      const listenerName = editingBackend
        ? editingBackend.listener.name || "unnamed"
        : form.selectedListenerName;

      // Find the target bind and route
      const bindIndex = config.binds.findIndex((b) => b.port === bindPort);
      if (bindIndex === -1) {
        throw new Error("Selected bind not found");
      }

      const listenerIndex = config.binds[bindIndex].listeners.findIndex(
        (l) => (l.name || "unnamed") === listenerName
      );
      if (listenerIndex === -1) {
        throw new Error("Selected listener not found");
      }

      if (!config.binds[bindIndex].listeners[listenerIndex].routes?.[routeIndex]) {
        throw new Error("Selected route not found");
      }

      // Create the new backend
      const newBackend = createBackendFromForm(form, backendType);

      if (editingBackend) {
        // Edit existing backend
        const route = config.binds[bindIndex].listeners[listenerIndex].routes![routeIndex];
        if (route.backends) {
          route.backends[editingBackend.backendIndex] = newBackend;
        }
      } else {
        // Add new backend
        if (!config.binds[bindIndex].listeners[listenerIndex].routes![routeIndex].backends) {
          config.binds[bindIndex].listeners[listenerIndex].routes![routeIndex].backends = [];
        }
        config.binds[bindIndex].listeners[listenerIndex].routes![routeIndex].backends.push(
          newBackend
        );
      }

      // Update the configuration
      await updateConfig(config);
      await refreshListeners();
      onSuccess();

      toast.success(
        `${backendType.toUpperCase()} backend "${form.name}" ${editingBackend ? "updated" : "added"} successfully`
      );
    } catch (err) {
      console.error(`Error ${editingBackend ? "updating" : "adding"} backend:`, err);

      const errorMessage = err instanceof Error ? err.message : String(err);
      if (errorMessage.includes("backend policies currently only work with exactly 1 backend")) {
        toast.error(
          `Cannot ${editingBackend ? "update" : "add"} backend: Backend policies require exactly 1 backend.`
        );
      } else {
        toast.error(`Failed to ${editingBackend ? "update" : "add"} backend`);
      }
    } finally {
      setIsSubmitting(false);
    }
  };

  const deleteBackend = async (backendContext: BackendWithContext, onSuccess: () => void) => {
    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find and remove the backend
      const bind = config.binds.find((b) => b.port === backendContext.bind.port);
      const listener = bind?.listeners.find((l) => l.name === backendContext.listener.name);
      const route = listener?.routes?.[backendContext.routeIndex];

      if (!route?.backends) {
        throw new Error("Route or backends not found");
      }

      const deleteCheck = canDeleteBackend(route, route.backends.length);
      if (!deleteCheck.canDelete) {
        throw new Error(deleteCheck.reason);
      }

      route.backends.splice(backendContext.backendIndex, 1);

      await updateConfig(config);
      await refreshListeners();
      onSuccess();

      toast.success("Backend deleted successfully");
    } catch (err) {
      console.error("Error deleting backend:", err);

      const errorMessage = err instanceof Error ? err.message : String(err);
      if (errorMessage.includes("backend policies currently only work with exactly 1 backend")) {
        toast.error(
          "Cannot delete backend: Backend policies require exactly 1 backend. Please remove backend policies first."
        );
      } else if (errorMessage.includes("backend policies")) {
        toast.error(errorMessage);
      } else {
        toast.error("Failed to delete backend");
      }
    } finally {
      setIsSubmitting(false);
    }
  };

  return {
    isSubmitting,
    addBackend,
    deleteBackend,
  };
};
