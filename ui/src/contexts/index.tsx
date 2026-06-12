// Export all contexts and their hooks
export * from "./LoadingContext";
export * from "./ServerContext";
export * from "./ThemeContext";
export * from "./WizardContext";

import type { ReactNode } from "react";
import React from "react";
import { LoadingProvider } from "./LoadingContext";
import { ServerProvider } from "./ServerContext";
import { ThemeProvider } from "./ThemeContext";
import { WizardProvider } from "./WizardContext";

// Combined AppProvider that wraps all context providers
export const AppProvider: React.FC<{ children: ReactNode }> = ({
  children,
}) => {
  return (
    <ThemeProvider>
      <LoadingProvider>
        <ServerProvider>
          <WizardProvider>{children}</WizardProvider>
        </ServerProvider>
      </LoadingProvider>
    </ThemeProvider>
  );
};
