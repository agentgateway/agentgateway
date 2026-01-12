"use client";

import { useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Plus, Route } from "lucide-react";
import {
  useRouteData,
  useRouteFormState,
  useRouteDialogs,
  useRouteOperations,
} from "@/lib/route-hooks";
import { populateEditForm, populateTcpEditForm } from "@/lib/route-utils";
import {
  RouteTable,
  AddRouteDialog,
  EditRouteDialog,
  EditTcpRouteDialog,
} from "@/components/route/route-components";
import { useXdsMode } from "@/hooks/use-xds-mode";

export function RouteConfig() {
  const {
    routes,
    tcpRoutes,
    isLoading,
    expandedBinds,
    setExpandedBinds,
    loadRoutes,
    getAvailableListeners,
    getAllRoutesByBind,
  } = useRouteData();

  const {
    routeForm,
    setRouteForm,
    tcpRouteForm,
    setTcpRouteForm,
    selectedListener,
    setSelectedListener,
    resetRouteForm,
  } = useRouteFormState();

  const {
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
  } = useRouteDialogs();

  const { isSubmitting, addRoute, editRoute, editTcpRoute, deleteRoute, deleteTcpRoute } =
    useRouteOperations();
  const xds = useXdsMode();

  // Load routes on component mount
  useEffect(() => {
    loadRoutes();
  }, [loadRoutes]);

  // Event handlers
  const handleAddRoute = async () => {
    if (!selectedListener) return;

    await addRoute(selectedListener, routeForm, tcpRouteForm, () => {
      loadRoutes();
      resetRouteForm();
      setIsAddRouteDialogOpen(false);
    });
  };

  const handleEditRoute = async () => {
    if (!editingRoute) return;

    await editRoute(editingRoute, routeForm, () => {
      loadRoutes();
      resetRouteForm();
      setEditingRoute(null);
      setIsEditRouteDialogOpen(false);
    });
  };

  const handleEditTcpRoute = async () => {
    if (!editingTcpRoute) return;

    await editTcpRoute(editingTcpRoute, tcpRouteForm, () => {
      loadRoutes();
      resetRouteForm();
      setEditingTcpRoute(null);
      setIsEditTcpRouteDialogOpen(false);
    });
  };

  const handleDeleteRoute = async (routeContext: (typeof routes)[0]) => {
    await deleteRoute(routeContext, () => {
      loadRoutes();
    });
  };

  const handleDeleteTcpRoute = async (tcpRouteContext: (typeof tcpRoutes)[0]) => {
    await deleteTcpRoute(tcpRouteContext, () => {
      loadRoutes();
    });
  };

  const handleEditRouteClick = (routeContext: (typeof routes)[0]) => {
    setEditingRoute(routeContext);
    const formData = populateEditForm(routeContext.route);
    setRouteForm(formData);
    setIsEditRouteDialogOpen(true);
  };

  const handleEditTcpRouteClick = (tcpRouteContext: (typeof tcpRoutes)[0]) => {
    setEditingTcpRoute(tcpRouteContext);
    const formData = populateTcpEditForm(tcpRouteContext.route);
    setTcpRouteForm(formData);
    setIsEditTcpRouteDialogOpen(true);
  };

  const handleCancelDialogs = () => {
    closeAllDialogs();
    resetRouteForm();
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
      <div className="flex justify-end">
        <Button
          onClick={() => setIsAddRouteDialogOpen(true)}
          disabled={xds}
          className={xds ? "opacity-50 cursor-not-allowed" : undefined}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Route
        </Button>
      </div>

      {routes.length === 0 && tcpRoutes.length === 0 ? (
        <div className="text-center py-12 border rounded-md bg-muted/20">
          <Route className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
          <p className="text-muted-foreground">No routes configured. Add a route to get started.</p>
        </div>
      ) : (
        <RouteTable
          allRoutesByBind={getAllRoutesByBind()}
          expandedBinds={expandedBinds}
          setExpandedBinds={setExpandedBinds}
          onEditRoute={handleEditRouteClick}
          onEditTcpRoute={handleEditTcpRouteClick}
          onDeleteRoute={handleDeleteRoute}
          onDeleteTcpRoute={handleDeleteTcpRoute}
          xds={xds}
        />
      )}

      <AddRouteDialog
        open={isAddRouteDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            handleCancelDialogs();
          }
          setIsAddRouteDialogOpen(open);
        }}
        routeForm={routeForm}
        setRouteForm={setRouteForm}
        tcpRouteForm={tcpRouteForm}
        setTcpRouteForm={setTcpRouteForm}
        selectedListener={selectedListener}
        setSelectedListener={setSelectedListener}
        availableListeners={getAvailableListeners()}
        onAddRoute={handleAddRoute}
        onCancel={handleCancelDialogs}
        isSubmitting={isSubmitting}
      />

      <EditRouteDialog
        open={isEditRouteDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            handleCancelDialogs();
          }
          setIsEditRouteDialogOpen(open);
        }}
        routeForm={routeForm}
        setRouteForm={setRouteForm}
        editingRoute={editingRoute}
        onEditRoute={handleEditRoute}
        onCancel={handleCancelDialogs}
        isSubmitting={isSubmitting}
        xds={xds}
      />

      <EditTcpRouteDialog
        open={isEditTcpRouteDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            handleCancelDialogs();
          }
          setIsEditTcpRouteDialogOpen(open);
        }}
        tcpRouteForm={tcpRouteForm}
        setTcpRouteForm={setTcpRouteForm}
        editingTcpRoute={editingTcpRoute}
        onEditTcpRoute={handleEditTcpRoute}
        onCancel={handleCancelDialogs}
        isSubmitting={isSubmitting}
        xds={xds}
      />
    </div>
  );
}
