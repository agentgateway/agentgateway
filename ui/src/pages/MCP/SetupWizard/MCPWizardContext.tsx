import type { ReactNode } from "react";
import React, { createContext, useCallback, useContext, useState } from "react";

export type MCPWizardStep = "selectServer" | /* "install" | */ "config";

export const MCP_WIZARD_STEPS: MCPWizardStep[] = [
    "selectServer",
    // "install",  // streamableHttp only — uncomment when re-enabling
    "config",
];

export interface MCPServerFields {
    name: string;
    args: string;
    // host: string;  // streamableHttp only
    // port: number;  // streamableHttp only
    // path: string;  // streamableHttp only
}

export interface MCPWizardData {
    port: number | null;
    selectedServer: string | null;
    // setupVerified: boolean;          // streamableHttp only
    // setupVerifyError: string | null; // streamableHttp only
    serverFields: MCPServerFields;
}

const DEFAULT_SERVER_FIELDS: MCPServerFields = {
    name: "my-server-everything",
    args: "@modelcontextprotocol/server-everything",
    // host: "localhost",  // streamableHttp only
    // port: 3001,         // streamableHttp only
    // path: "/mcp",       // streamableHttp only
};

const DEFAULT_DATA: MCPWizardData = {
    port: 8622,
    selectedServer: null,
    // setupVerified: false,       // streamableHttp only
    // setupVerifyError: null,     // streamableHttp only
    serverFields: DEFAULT_SERVER_FIELDS,
};

interface MCPWizardContextType { 
    currentStep: MCPWizardStep;
    stepIndex: number;
    data: MCPWizardData;
    nextStep: () => void;
    previousStep: () => void;
    setPort: (port: number) => void;
    setSelectedServer: (server: string) => void;
    // setVerified: (verified: boolean, error?: string | null) => void;  // streamableHttp only
    updateServerFields: (fields: Partial<MCPServerFields>) => void;
    canGoNext: boolean;
    canGoPrevious: boolean;
}

const MCPWizardContext = createContext<MCPWizardContextType | undefined>(undefined);

export const MCPWizardProvider: React.FC<{ children: ReactNode }> = ({ children }) => {
    const [currentStep, setCurrentStep] = useState<MCPWizardStep>("selectServer");
    const [data, setData] = useState<MCPWizardData>(DEFAULT_DATA);

    const stepIndex = MCP_WIZARD_STEPS.indexOf(currentStep);
    const canGoNext = stepIndex < MCP_WIZARD_STEPS.length - 1;
    const canGoPrevious = stepIndex > 0;

    const nextStep = useCallback(() => {
        if (canGoNext) setCurrentStep(MCP_WIZARD_STEPS[stepIndex + 1]);
    }, [stepIndex, canGoNext]);

    const previousStep = useCallback(() => {
        if (canGoPrevious) setCurrentStep(MCP_WIZARD_STEPS[stepIndex - 1]);
    }, [stepIndex, canGoPrevious]);

    const setPort = useCallback((port: number) => {
        setData((prev) => ({ ...prev, port }));
    }, []);

    const setSelectedServer = useCallback((server: string) => {
        setData((prev) => ({
            ...prev,
            selectedServer: server,
            setupVerified: false,
            setupVerifyError: null,
        }));
    }, []);

    // const setVerified = useCallback((verified: boolean, error: string | null = null) => {  // streamableHttp only
    //     setData((prev) => ({ ...prev, setupVerified: verified, setupVerifyError: error }));
    // }, []);

    const updateServerFields = useCallback((fields: Partial<MCPServerFields>) => {
        setData((prev) => ({ ...prev, serverFields: { ...prev.serverFields, ...fields } }));
    }, []);

    return (
        <MCPWizardContext.Provider
            value={{
                currentStep,
                stepIndex,
                data,
                nextStep,
                previousStep,
                setPort,
                setSelectedServer: setSelectedServer,
                // setVerified,  // streamableHttp only
                updateServerFields,
                canGoNext,
                canGoPrevious,
            }}
        >
            {children}
        </MCPWizardContext.Provider>
    );
};

export const useMCPWizard = (): MCPWizardContextType => { 
    const context = useContext(MCPWizardContext);
    if (!context) throw new Error("useMCPWizard mus tbe used within MCPWizardProvider");
    return context;
}