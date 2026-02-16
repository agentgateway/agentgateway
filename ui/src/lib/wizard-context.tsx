import { createContext, useContext, useState, ReactNode, useEffect } from "react";
import { useServer } from "@/lib/server-context";
import { useLoading } from "@/lib/loading-context";
import { deleteEverything } from "@/lib/api";

interface WizardContextType {
  isWizardVisible: boolean;
  showWizard: boolean;
  wizardStarted: boolean;
  showSetupWizard: () => void;
  hideSetupWizard: () => void;
  handleWizardComplete: () => void;
  handleWizardSkip: () => void;
  checkAndShowWizardIfNeeded: () => Promise<void>;
  restartWizard: () => Promise<void>;
  isRestartingWizard: boolean;
}

const WizardContext = createContext<WizardContextType | undefined>(undefined);

export function WizardProvider({ children }: { children: ReactNode }) {
  const [isWizardVisible, setIsWizardVisible] = useState(false);
  const [showWizard, setShowWizard] = useState(false);
  const [wizardStarted, setWizardStarted] = useState(false);
  const [isRestartingWizard, setIsRestartingWizard] = useState(false);
  const { isConfigurationEmpty } = useServer();
  const { setIsLoading } = useLoading();

  // Effect to handle manual wizard restart through storage events
  useEffect(() => {
    const handleStorageChange = (e: StorageEvent) => {
      if (e.key === "agentgateway.setupCompleted" && e.newValue === "false") {
        showSetupWizard();
      }
    };

    window.addEventListener("storage", handleStorageChange);
    return () => window.removeEventListener("storage", handleStorageChange);
  }, []);

  const showSetupWizard = () => {
    setShowWizard(true);
    setWizardStarted(true);
    setIsWizardVisible(true);
  };

  const hideSetupWizard = () => {
    setShowWizard(false);
    setWizardStarted(false);
    setIsWizardVisible(false);
  };

  const handleWizardComplete = () => {
    localStorage.setItem("agentgateway.setupCompleted", "true");
    hideSetupWizard();
  };

  const handleWizardSkip = () => {
    localStorage.setItem("agentgateway.setupCompleted", "true");
    hideSetupWizard();
  };

  const checkAndShowWizardIfNeeded = async () => {
    const setupCompleted = localStorage.getItem("agentgateway.setupCompleted");
    const isEmpty = await isConfigurationEmpty();

    if (isEmpty && (setupCompleted === null || setupCompleted === "false")) {
      showSetupWizard();
    }
  };

  const restartWizard = async () => {
    try {
      setIsRestartingWizard(true);
      setIsLoading(true);
      await deleteEverything();
      localStorage.setItem("agentgateway.setupCompleted", "false");
      showSetupWizard();
    } catch (error) {
      console.error("Error restarting wizard:", error);
      throw error; // Re-throw to let the UI handle the error
    } finally {
      setIsRestartingWizard(false);
      setIsLoading(false);
    }
  };

  // Effect to check configuration on mount and when isConfigurationEmpty changes
  useEffect(() => {
    checkAndShowWizardIfNeeded();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isConfigurationEmpty]);

  return (
    <WizardContext.Provider
      value={{
        isWizardVisible,
        showWizard,
        wizardStarted,
        showSetupWizard,
        hideSetupWizard,
        handleWizardComplete,
        handleWizardSkip,
        checkAndShowWizardIfNeeded,
        restartWizard,
        isRestartingWizard,
      }}
    >
      {children}
    </WizardContext.Provider>
  );
}

export function useWizard() {
  const context = useContext(WizardContext);
  if (context === undefined) {
    throw new Error("useWizard must be used within a WizardProvider");
  }
  return context;
}
