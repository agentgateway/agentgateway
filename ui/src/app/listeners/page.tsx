"use client";

import { ListenerConfig } from "@/components/listener-config";
import { useServer } from "@/lib/server-context";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { AlertCircle, Network } from "lucide-react";
import { useState } from "react";

export default function ListenersPage() {
  const { connectionError, binds } = useServer();
  const [isAddingListener, setIsAddingListener] = useState(false);

  const getTotalListeners = () => {
    return binds?.reduce((total, bind) => total + bind.listeners.length, 0) || 0;
  };

  return (
    <div className="container mx-auto py-8 px-4">
      <div className="flex flex-row items-center justify-between mb-6">
        <div>
          <div className="flex items-center space-x-3">
            <Network className="h-8 w-8 text-primary" />
            <div>
              <h1 className="text-3xl font-bold tracking-tight">Port Binds & Listeners</h1>
              <p className="text-muted-foreground mt-1">
                Configure port bindings and manage listeners for your gateway
              </p>
            </div>
          </div>
          {binds && binds.length > 0 && (
            <div className="mt-4 flex items-center space-x-6 text-sm text-muted-foreground">
              <div className="flex items-center space-x-2">
                <div className="w-2 h-2 bg-primary rounded-full"></div>
                <span>
                  {binds.length} port bind{binds.length !== 1 ? "s" : ""}
                </span>
              </div>
              <div className="flex items-center space-x-2">
                <div className="w-2 h-2 bg-green-500 rounded-full"></div>
                <span>
                  {getTotalListeners()} listener{getTotalListeners() !== 1 ? "s" : ""}
                </span>
              </div>
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
        <ListenerConfig
          isAddingListener={isAddingListener}
          setIsAddingListener={setIsAddingListener}
        />
      )}
    </div>
  );
}
