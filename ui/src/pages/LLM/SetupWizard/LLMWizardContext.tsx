import type { ReactNode } from "react";
import React, { createContext, useCallback, useContext, useState } from "react";

export type LLMWizardStep = "selectModel" | "install" | "modelConfig";

export const LLM_WIZARD_STEPS: LLMWizardStep[] = [
    "selectModel",
    "install",
    "modelConfig",
];

export interface LLMModelFields { 
    name: string;
    provider: string;
    model: string;
    hostOverride: string;
};

export interface LLMWizardData {
    selectedWalkthrough: string | null;
    setupVerified: boolean;
    setupVerifyError: string | null;
    modelFields: LLMModelFields;
};

const DEFAULT_MODEL_FIELDS: LLMModelFields = {
    name: "my-ollama-smallthinker",
    provider: "openAI",
    model: "smallthinker",
    hostOverride: "http://localhost:11434",
};

const DEFAULT_DATA: LLMWizardData = {
    selectedWalkthrough: null,
    setupVerified: false,
    setupVerifyError: null,
    modelFields: DEFAULT_MODEL_FIELDS,
};

interface LLMWizardContextType {
    currentStep: LLMWizardStep;
    stepIndex: number;
    totalSteps: number;
    data: LLMWizardData;
    nextStep: () => void;
    previousStep: () => void;
    goToStep: (step: LLMWizardStep) => void;
    setSelectedWalkthrough: (walkthrough: string) => void;
    setWalkthroughVerified: (verified: boolean, error?: string | null) => void;
    updateModelFields: (fields: Partial<LLMModelFields>) => void;
    resetWizard: () => void;
    canGoNext: boolean;
    canGoPrevious: boolean;
}

const LLMWizardContext = createContext<LLMWizardContextType | undefined>(undefined);

export const LLMWizardProvider: React.FC<{ children: ReactNode }> = ({ children }) => { 
    const [currentStep, setCurrentStep] = useState<LLMWizardStep>("selectModel");
    const [data, setData] = useState<LLMWizardData>(DEFAULT_DATA);

    const stepIndex = LLM_WIZARD_STEPS.indexOf(currentStep);
    const totalSteps = LLM_WIZARD_STEPS.length;
    const canGoNext = stepIndex < totalSteps - 1;
    const canGoPrevious = stepIndex > 0;

    const nextStep = useCallback(() => { 
        if (canGoNext) { 
            setCurrentStep(LLM_WIZARD_STEPS[stepIndex + 1]);
        }
    }, [stepIndex, canGoNext]);

    const previousStep = useCallback(() => {
        if (canGoPrevious) {
            setCurrentStep(LLM_WIZARD_STEPS[stepIndex - 1]);
        }
    }, [stepIndex, canGoPrevious]);

    const goToStep = useCallback((step: LLMWizardStep) => { 
        if (LLM_WIZARD_STEPS.includes(step)) {
            setCurrentStep(step);
        }
    }, []);

    const setSelectedWalkthrough = useCallback((walkthrough: string) => { 
        setData((prev) => ({
            ...prev,
            selectedWalkthrough: walkthrough,
            setupVerified: false,
            setupVerifyError: null,
        }));
    }, []);

    const setWalkthroughVerified = useCallback((verified: boolean, error: string | null = null) => { 
        setData((prev) => ({
            ...prev,
            setupVerified: verified,
            setupVerifyError: error,
        }));
    }, []);

    const updateModelFields = useCallback((fields: Partial<LLMModelFields>) => { 
        setData((prev) => ({
            ...prev,
            modelFields: { ...prev.modelFields, ...fields },
        }))
    }, []);

    const resetWizard = useCallback(() => { 
        setCurrentStep("selectModel");
        setData(DEFAULT_DATA);
    }, []);

    return (
        <LLMWizardContext.Provider
            value={{
                currentStep,
                stepIndex,
                totalSteps,
                data,
                nextStep,
                previousStep,
                goToStep,
                setSelectedWalkthrough,
                setWalkthroughVerified,
                updateModelFields,
                resetWizard,
                canGoNext,
                canGoPrevious,
            }}
        >
            {children}
        </LLMWizardContext.Provider>
    )
};

export const useLLMWizard = (): LLMWizardContextType => { 
    const context = useContext(LLMWizardContext);
    if (!context) throw new Error("useLLMWizard must be used within an LLMWizardProvider");
    return context;
}