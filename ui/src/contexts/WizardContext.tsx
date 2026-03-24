import type { ReactNode } from "react";
import React, { createContext, useCallback, useContext, useState } from "react";

// Wizard steps
export type WizardStep =
  | "welcome"
  | "listener"
  | "routes"
  | "backends"
  | "policies"
  | "review";

export const WIZARD_STEPS: WizardStep[] = [
  "welcome",
  "listener",
  "routes",
  "backends",
  "policies",
  "review",
];

// Wizard data collected throughout the flow
export interface WizardData {
  listener?: {
    name: string;
    port: number;
    protocol: string;
  };
  routes?: Array<{
    name: string;
    path: string;
    backend: string;
  }>;
  backends?: Array<{
    name: string;
    type: string;
    target: string;
  }>;
  policies?: Array<{
    name: string;
    type: string;
    config: Record<string, unknown>;
  }>;
}

interface WizardContextType {
  currentStep: WizardStep;
  stepIndex: number;
  totalSteps: number;
  data: WizardData;
  completed: boolean;
  setCurrentStep: (step: WizardStep) => void;
  nextStep: () => void;
  previousStep: () => void;
  goToStep: (step: WizardStep) => void;
  updateData: (data: Partial<WizardData>) => void;
  resetWizard: () => void;
  completeWizard: () => void;
  canGoNext: boolean;
  canGoPrevious: boolean;
}

const WizardContext = createContext<WizardContextType | undefined>(undefined);

export const WizardProvider: React.FC<{ children: ReactNode }> = ({
  children,
}) => {
  const [currentStep, setCurrentStep] = useState<WizardStep>("welcome");
  const [data, setData] = useState<WizardData>({});
  const [completed, setCompleted] = useState(false);

  const stepIndex = WIZARD_STEPS.indexOf(currentStep);
  const totalSteps = WIZARD_STEPS.length;
  const canGoNext = stepIndex < totalSteps - 1;
  const canGoPrevious = stepIndex > 0;

  const nextStep = useCallback(() => {
    if (canGoNext) {
      setCurrentStep(WIZARD_STEPS[stepIndex + 1]);
    }
  }, [stepIndex, canGoNext]);

  const previousStep = useCallback(() => {
    if (canGoPrevious) {
      setCurrentStep(WIZARD_STEPS[stepIndex - 1]);
    }
  }, [stepIndex, canGoPrevious]);

  const goToStep = useCallback((step: WizardStep) => {
    if (WIZARD_STEPS.includes(step)) {
      setCurrentStep(step);
    }
  }, []);

  const updateData = useCallback((newData: Partial<WizardData>) => {
    setData((prev) => ({ ...prev, ...newData }));
  }, []);

  const resetWizard = useCallback(() => {
    setCurrentStep("welcome");
    setData({});
    setCompleted(false);
  }, []);

  const completeWizard = useCallback(() => {
    setCompleted(true);
  }, []);

  return (
    <WizardContext.Provider
      value={{
        currentStep,
        stepIndex,
        totalSteps,
        data,
        completed,
        setCurrentStep,
        nextStep,
        previousStep,
        goToStep,
        updateData,
        resetWizard,
        completeWizard,
        canGoNext,
        canGoPrevious,
      }}
    >
      {children}
    </WizardContext.Provider>
  );
};

export const useWizard = (): WizardContextType => {
  const context = useContext(WizardContext);
  if (!context) {
    throw new Error("useWizard must be used within a WizardProvider");
  }
  return context;
};

// Helper hook to determine if a step is accessible
export const useWizardNavigation = () => {
  const { currentStep, stepIndex, data } = useWizard();

  const isStepAccessible = useCallback(
    (step: WizardStep): boolean => {
      // Welcome is always accessible
      if (step === "welcome") return true;

      // Can't go forward if current step isn't "complete"
      // For now, allow all steps (validation can be added later)
      return true;
    },
    [currentStep, stepIndex, data],
  );

  const isStepComplete = useCallback(
    (step: WizardStep): boolean => {
      switch (step) {
        case "welcome":
          return true;
        case "listener":
          return !!data.listener;
        case "routes":
          return !!data.routes && data.routes.length > 0;
        case "backends":
          return !!data.backends && data.backends.length > 0;
        case "policies":
          return true; // Policies are optional
        case "review":
          return false; // Review is never "complete" until wizard is done
        default:
          return false;
      }
    },
    [data],
  );

  return { isStepAccessible, isStepComplete };
};
