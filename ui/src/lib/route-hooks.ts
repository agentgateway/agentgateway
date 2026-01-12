import { useCallback, useState } from "react";
import { Route as RouteType, TcpRoute, Listener, Bind } from "@/lib/types";
import { fetchBinds, updateConfig, fetchConfig } from "@/lib/api";
import { useServer } from "@/lib/server-context";
import { toast } from "sonner";
import { DEFAULT_HTTP_ROUTE_FORM, DEFAULT_TCP_ROUTE_FORM } from "./route-constants";
import {
  isTcpListener,
  buildMatch,
  createHttpRoute,
  createTcpRoute,
  updateHttpRoute,
  updateTcpRoute,
} from "./route-utils";

export interface RouteWithContext {
  route: RouteType;
  listener: Listener;
  bind: Bind;
  routeIndex: number;
}

export interface TcpRouteWithContext {
  route: TcpRoute;
  listener: Listener;
  bind: Bind;
  routeIndex: number;
}

export interface CombinedRouteWithContext {
  type: "http" | "tcp";
  route: RouteType | TcpRoute;
  listener: Listener;
  bind: Bind;
  routeIndex: number;
}

// Hook for managing route data loading and organization
export const useRouteData = () => {
  const [binds, setBinds] = useState<Bind[]>([]);
  const [routes, setRoutes] = useState<RouteWithContext[]>([]);
  const [tcpRoutes, setTcpRoutes] = useState<TcpRouteWithContext[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [expandedBinds, setExpandedBinds] = useState<Set<number>>(new Set());

  const loadRoutes = useCallback(async () => {
    setIsLoading(true);
    try {
      const fetchedBinds = await fetchBinds();
      setBinds(fetchedBinds);

      // Extract all routes with context
      const allRoutes: RouteWithContext[] = [];
      const allTcpRoutes: TcpRouteWithContext[] = [];

      fetchedBinds.forEach((bind) => {
        bind.listeners.forEach((listener) => {
          listener.routes?.forEach((route, routeIndex) => {
            allRoutes.push({ route, listener, bind, routeIndex });
          });

          listener.tcpRoutes?.forEach((tcpRoute, routeIndex) => {
            allTcpRoutes.push({ route: tcpRoute, listener, bind, routeIndex });
          });
        });
      });

      setRoutes(allRoutes);
      setTcpRoutes(allTcpRoutes);

      // Auto-expand binds with routes
      const bindsWithRoutes = new Set<number>();
      allRoutes.forEach(({ bind }) => bindsWithRoutes.add(bind.port));
      allTcpRoutes.forEach(({ bind }) => bindsWithRoutes.add(bind.port));
      setExpandedBinds(bindsWithRoutes);
    } catch (err) {
      console.error("Error loading routes:", err);
      toast.error("Failed to load routes");
    } finally {
      setIsLoading(false);
    }
  }, []);

  const getAvailableListeners = (): { listener: Listener; bind: Bind }[] => {
    const listeners: { listener: Listener; bind: Bind }[] = [];
    binds.forEach((bind) => {
      bind.listeners.forEach((listener) => {
        listeners.push({ listener, bind });
      });
    });
    return listeners;
  };

  const getAllRoutesByBind = (): Map<number, CombinedRouteWithContext[]> => {
    const allRoutesByBind = new Map<number, CombinedRouteWithContext[]>();

    // Get all unique ports
    const allPorts = new Set<number>();
    routes.forEach((r) => allPorts.add(r.bind.port));
    tcpRoutes.forEach((r) => allPorts.add(r.bind.port));

    // Initialize each port with empty arrays
    allPorts.forEach((port) => {
      allRoutesByBind.set(port, []);
    });

    // Populate with HTTP routes
    routes.forEach((routeContext) => {
      const port = routeContext.bind.port;
      allRoutesByBind.get(port)!.push({
        type: "http",
        route: routeContext.route,
        listener: routeContext.listener,
        bind: routeContext.bind,
        routeIndex: routeContext.routeIndex,
      });
    });

    // Populate with TCP routes
    tcpRoutes.forEach((tcpRouteContext) => {
      const port = tcpRouteContext.bind.port;
      allRoutesByBind.get(port)!.push({
        type: "tcp",
        route: tcpRouteContext.route,
        listener: tcpRouteContext.listener,
        bind: tcpRouteContext.bind,
        routeIndex: tcpRouteContext.routeIndex,
      });
    });

    return allRoutesByBind;
  };

  return {
    binds,
    routes,
    tcpRoutes,
    isLoading,
    expandedBinds,
    setExpandedBinds,
    loadRoutes,
    getAvailableListeners,
    getAllRoutesByBind,
  };
};

// Hook for managing form state
export const useRouteFormState = () => {
  const [routeForm, setRouteForm] = useState(DEFAULT_HTTP_ROUTE_FORM);
  const [tcpRouteForm, setTcpRouteForm] = useState(DEFAULT_TCP_ROUTE_FORM);
  const [selectedListener, setSelectedListener] = useState<{
    listener: Listener;
    bind: Bind;
  } | null>(null);

  const resetRouteForm = () => {
    setRouteForm(DEFAULT_HTTP_ROUTE_FORM);
    setTcpRouteForm(DEFAULT_TCP_ROUTE_FORM);
    setSelectedListener(null);
  };

  return {
    routeForm,
    setRouteForm,
    tcpRouteForm,
    setTcpRouteForm,
    selectedListener,
    setSelectedListener,
    resetRouteForm,
  };
};

// Hook for managing dialog states
export const useRouteDialogs = () => {
  const [isAddRouteDialogOpen, setIsAddRouteDialogOpen] = useState(false);
  const [isEditRouteDialogOpen, setIsEditRouteDialogOpen] = useState(false);
  const [isEditTcpRouteDialogOpen, setIsEditTcpRouteDialogOpen] = useState(false);
  const [editingRoute, setEditingRoute] = useState<RouteWithContext | null>(null);
  const [editingTcpRoute, setEditingTcpRoute] = useState<TcpRouteWithContext | null>(null);

  const closeAllDialogs = () => {
    setIsAddRouteDialogOpen(false);
    setIsEditRouteDialogOpen(false);
    setIsEditTcpRouteDialogOpen(false);
    setEditingRoute(null);
    setEditingTcpRoute(null);
  };

  return {
    isAddRouteDialogOpen,
    setIsAddRouteDialogOpen,
    isEditRouteDialogOpen,
    setIsEditRouteDialogOpen,
    isEditTcpRouteDialogOpen,
    setIsEditTcpRouteDialogOpen,
    editingRoute,
    setEditingRoute,
    editingTcpRoute,
    setEditingTcpRoute,
    closeAllDialogs,
  };
};

// Hook for managing route operations (CRUD)
export const useRouteOperations = () => {
  const { refreshListeners } = useServer();
  const [isSubmitting, setIsSubmitting] = useState(false);

  const addRoute = async (
    selectedListener: { listener: Listener; bind: Bind },
    routeForm: typeof DEFAULT_HTTP_ROUTE_FORM,
    tcpRouteForm: typeof DEFAULT_TCP_ROUTE_FORM,
    onSuccess: () => void
  ) => {
    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find the specific bind and listener
      const bind = config.binds.find((b) => b.port === selectedListener.bind.port);
      const listener = bind?.listeners.find((l) => l.name === selectedListener.listener.name);

      if (!bind || !listener) {
        throw new Error("Could not find bind or listener");
      }

      if (isTcpListener(selectedListener.listener)) {
        // Ensure tcpRoutes array exists
        if (!listener.tcpRoutes) {
          listener.tcpRoutes = [];
        }

        const newTcpRoute = createTcpRoute(tcpRouteForm);
        listener.tcpRoutes.push(newTcpRoute);
      } else {
        // Ensure routes array exists
        if (!listener.routes) {
          listener.routes = [];
        }

        const match = buildMatch(routeForm);
        const newRoute = createHttpRoute(routeForm, match);
        listener.routes.push(newRoute);
      }

      await updateConfig(config);
      await refreshListeners();
      onSuccess();

      const routeType = isTcpListener(selectedListener.listener) ? "TCP" : "HTTP";
      toast.success(`${routeType} route added successfully`);
    } catch (err) {
      console.error("Error adding route:", err);
      toast.error(err instanceof Error ? err.message : "Failed to add route");
    } finally {
      setIsSubmitting(false);
    }
  };

  const editRoute = async (
    editingRoute: RouteWithContext,
    routeForm: typeof DEFAULT_HTTP_ROUTE_FORM,
    onSuccess: () => void
  ) => {
    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find the specific bind, listener, and route
      const bind = config.binds.find((b) => b.port === editingRoute.bind.port);
      const listener = bind?.listeners.find((l) => l.name === editingRoute.listener.name);

      if (!bind || !listener || !listener.routes) {
        throw new Error("Could not find bind, listener, or routes");
      }

      const match = buildMatch(routeForm);
      const updatedRoute = updateHttpRoute(routeForm, match, editingRoute.route);

      // Replace the route at the specific index
      listener.routes[editingRoute.routeIndex] = updatedRoute;

      await updateConfig(config);
      await refreshListeners();
      onSuccess();

      toast.success("Route updated successfully");
    } catch (err) {
      console.error("Error updating route:", err);
      toast.error(err instanceof Error ? err.message : "Failed to update route");
    } finally {
      setIsSubmitting(false);
    }
  };

  const editTcpRoute = async (
    editingTcpRoute: TcpRouteWithContext,
    tcpRouteForm: typeof DEFAULT_TCP_ROUTE_FORM,
    onSuccess: () => void
  ) => {
    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find the specific bind, listener, and TCP route
      const bind = config.binds.find((b) => b.port === editingTcpRoute.bind.port);
      const listener = bind?.listeners.find((l) => l.name === editingTcpRoute.listener.name);

      if (!bind || !listener || !listener.tcpRoutes) {
        throw new Error("Could not find bind, listener, or TCP routes");
      }

      const updatedTcpRoute = updateTcpRoute(tcpRouteForm, editingTcpRoute.route);

      // Replace the TCP route at the specific index
      listener.tcpRoutes[editingTcpRoute.routeIndex] = updatedTcpRoute;

      await updateConfig(config);
      await refreshListeners();
      onSuccess();

      toast.success("TCP route updated successfully");
    } catch (err) {
      console.error("Error updating TCP route:", err);
      toast.error(err instanceof Error ? err.message : "Failed to update TCP route");
    } finally {
      setIsSubmitting(false);
    }
  };

  const deleteRoute = async (routeContext: RouteWithContext, onSuccess: () => void) => {
    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find and remove the route
      const bind = config.binds.find((b) => b.port === routeContext.bind.port);
      const listener = bind?.listeners.find((l) => l.name === routeContext.listener.name);

      if (listener?.routes) {
        listener.routes.splice(routeContext.routeIndex, 1);
      }

      await updateConfig(config);
      await refreshListeners();
      onSuccess();

      toast.success("Route deleted successfully");
    } catch (err) {
      console.error("Error deleting route:", err);
      toast.error("Failed to delete route");
    } finally {
      setIsSubmitting(false);
    }
  };

  const deleteTcpRoute = async (tcpRouteContext: TcpRouteWithContext, onSuccess: () => void) => {
    setIsSubmitting(true);
    try {
      const config = await fetchConfig();

      // Find and remove the TCP route
      const bind = config.binds.find((b) => b.port === tcpRouteContext.bind.port);
      const listener = bind?.listeners.find((l) => l.name === tcpRouteContext.listener.name);

      if (listener?.tcpRoutes) {
        listener.tcpRoutes.splice(tcpRouteContext.routeIndex, 1);
      }

      await updateConfig(config);
      await refreshListeners();
      onSuccess();

      toast.success("TCP route deleted successfully");
    } catch (err) {
      console.error("Error deleting TCP route:", err);
      toast.error("Failed to delete TCP route");
    } finally {
      setIsSubmitting(false);
    }
  };

  return {
    isSubmitting,
    addRoute,
    editRoute,
    editTcpRoute,
    deleteRoute,
    deleteTcpRoute,
  };
};
