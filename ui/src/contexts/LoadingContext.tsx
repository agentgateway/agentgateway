import type { ReactNode } from "react";
import React, { createContext, useCallback, useContext, useState } from "react";

// Loading Context
interface LoadingContextType {
  loading: boolean;
  setLoading: (loading: boolean) => void;
  loadingMessage?: string;
  setLoadingMessage: (message?: string) => void;
}

const LoadingContext = createContext<LoadingContextType | undefined>(undefined);

export const LoadingProvider: React.FC<{ children: ReactNode }> = ({
  children,
}) => {
  const [loading, setLoading] = useState(false);
  const [loadingMessage, setLoadingMessage] = useState<string | undefined>();

  return (
    <LoadingContext.Provider
      value={{
        loading,
        setLoading,
        loadingMessage,
        setLoadingMessage,
      }}
    >
      {children}
    </LoadingContext.Provider>
  );
};

export const useLoading = (): LoadingContextType => {
  const context = useContext(LoadingContext);
  if (!context) {
    throw new Error("useLoading must be used within a LoadingProvider");
  }
  return context;
};

// Hook for easy loading state management
export const useLoadingState = () => {
  const { setLoading, setLoadingMessage } = useLoading();

  const startLoading = useCallback(
    (message?: string) => {
      setLoadingMessage(message);
      setLoading(true);
    },
    [setLoading, setLoadingMessage],
  );

  const stopLoading = useCallback(() => {
    setLoading(false);
    setLoadingMessage(undefined);
  }, [setLoading, setLoadingMessage]);

  return { startLoading, stopLoading };
};
