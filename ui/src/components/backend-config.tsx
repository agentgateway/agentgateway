"use client";

import { useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Plus, Target } from "lucide-react";
import {
  useBackendData,
  useBackendFormState,
  useBackendDialogs,
  useBackendOperations,
} from "@/lib/backend-hooks";
import { BackendTable, AddBackendDialog } from "@/components/backend/backend-components";
import { useXdsMode } from "@/hooks/use-xds-mode";

export function BackendConfig() {
  const {
    binds,
    backends,
    isLoading,
    expandedBinds,
    setExpandedBinds,
    loadBackends,
    getBackendsByBind,
  } = useBackendData();

  const {
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
  } = useBackendFormState();

  const {
    isAddBackendDialogOpen,
    setIsAddBackendDialogOpen,
    editingBackend,
    openAddDialog,
    openEditDialog,
    closeDialogs,
  } = useBackendDialogs();

  const { isSubmitting, addBackend, deleteBackend } = useBackendOperations();
  const xds = useXdsMode();

  // Load backends on component mount
  useEffect(() => {
    loadBackends();
  }, [loadBackends]);

  // Event handlers
  const handleAddBackend = async () => {
    await addBackend(backendForm, selectedBackendType, editingBackend, () => {
      loadBackends();
      resetBackendForm(binds);
      closeDialogs();
    });
  };

  const handleEditBackend = (backendContext: (typeof backends)[0]) => {
    populateFormFromBackendContext(backendContext);
    openEditDialog(backendContext);
  };

  const handleDeleteBackend = async (backendContext: (typeof backends)[0]) => {
    await deleteBackend(backendContext, () => {
      loadBackends();
    });
  };

  const handleCancel = () => {
    closeDialogs();
    resetBackendForm(binds);
  };

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-8">
        <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary"></div>
        <span className="ml-2">Loading backends...</span>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-end">
        <Button
          onClick={() => {
            resetBackendForm(binds);
            openAddDialog();
          }}
          disabled={xds}
          className={xds ? "opacity-50 cursor-not-allowed" : undefined}
        >
          <Plus className="mr-2 h-4 w-4" />
          Add Backend
        </Button>
      </div>

      {backends.length === 0 ? (
        <div className="text-center py-12 border rounded-md bg-muted/20">
          <Target className="mx-auto h-12 w-12 text-muted-foreground mb-4" />
          <p className="text-muted-foreground">No backends configured.</p>
          <p className="text-sm text-muted-foreground mt-2">
            Add backends to your routes to get started.
          </p>
        </div>
      ) : (
        <BackendTable
          backendsByBind={getBackendsByBind()}
          expandedBinds={expandedBinds}
          setExpandedBinds={setExpandedBinds}
          onEditBackend={handleEditBackend}
          onDeleteBackend={handleDeleteBackend}
          isSubmitting={isSubmitting}
        />
      )}

      <AddBackendDialog
        open={isAddBackendDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            handleCancel();
          }
          setIsAddBackendDialogOpen(open);
        }}
        binds={binds}
        backendForm={backendForm}
        setBackendForm={setBackendForm}
        selectedBackendType={selectedBackendType}
        setSelectedBackendType={setSelectedBackendType}
        editingBackend={editingBackend}
        onAddBackend={handleAddBackend}
        onCancel={handleCancel}
        isSubmitting={isSubmitting}
        addMcpTarget={addMcpTarget}
        removeMcpTarget={removeMcpTarget}
        updateMcpTarget={updateMcpTarget}
        parseAndUpdateUrl={parseAndUpdateUrl}
        updateMcpStateful={updateMcpStateful}
      />
    </div>
  );
}
