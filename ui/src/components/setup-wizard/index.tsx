"use client";

import { useState } from "react";
import { LocalConfig } from "@/lib/types";
import { WelcomeStep } from "./WelcomeStep";
import { ListenerStep } from "./ListenerStep";
import { RouteStep } from "./RouteStep";
import { BackendStep } from "./BackendStep";
import { PolicyStep } from "./PolicyStep";
import { ReviewStep } from "./ReviewStep";

interface SetupWizardProps {
  config: LocalConfig;
  onConfigChange: (config: LocalConfig) => void;
  onComplete: () => void;
  onSkip: () => void;
}

export function SetupWizard({ config, onConfigChange, onComplete, onSkip }: SetupWizardProps) {
  const [step, setStep] = useState(1);
  const totalSteps = 6;

  const renderStep = () => {
    switch (step) {
      case 1:
        return <WelcomeStep onNext={() => setStep(2)} onSkip={onSkip} />;
      case 2:
        return (
          <ListenerStep
            onNext={() => setStep(3)}
            onPrevious={() => setStep(1)}
            config={config}
            onConfigChange={onConfigChange}
          />
        );
      case 3:
        return (
          <RouteStep
            onNext={() => setStep(4)}
            onPrevious={() => setStep(2)}
            config={config}
            onConfigChange={onConfigChange}
          />
        );
      case 4:
        return (
          <BackendStep
            onNext={() => setStep(5)}
            onPrevious={() => setStep(3)}
            config={config}
            onConfigChange={onConfigChange}
          />
        );
      case 5:
        return (
          <PolicyStep
            onNext={() => setStep(6)}
            onPrevious={() => setStep(4)}
            config={config}
            onConfigChange={onConfigChange}
          />
        );
      case 6:
        return (
          <ReviewStep
            onNext={onComplete}
            onPrevious={() => setStep(5)}
            config={config}
            onConfigChange={onConfigChange}
          />
        );
      default:
        return null;
    }
  };

  const getStepLabel = (stepNumber: number) => {
    const labels = ["Welcome", "Listener", "Routes", "Backends", "Policies", "Review"];
    return labels[stepNumber - 1];
  };

  return (
    <div
      className="fixed inset-0 flex items-center justify-center bg-gradient-to-br from-background via-background/95 to-muted/30"
      data-cy="setup-wizard-container"
    >
      <div className="w-full max-w-4xl px-4">
        {renderStep()}

        {/* Progress indicator */}
        <div className="flex justify-center mt-6" data-cy="wizard-progress-indicator">
          <div className="flex items-center">
            {Array.from({ length: totalSteps }).map((_, i) => {
              const stepNumber = i + 1;
              const isActive = stepNumber === step;
              const isCompleted = stepNumber < step;

              return (
                <div key={i} className="flex items-center">
                  <div className="flex flex-col items-center">
                    <div
                      className={`h-8 w-8 rounded-full flex items-center justify-center text-xs font-medium transition-colors ${
                        isActive
                          ? "bg-primary text-primary-foreground"
                          : isCompleted
                            ? "bg-primary/20 text-primary"
                            : "bg-muted text-muted-foreground"
                      }`}
                      data-cy={`wizard-step-${stepNumber}`}
                    >
                      {stepNumber}
                    </div>
                    <span
                      className={`text-xs mt-1 whitespace-nowrap ${
                        isActive ? "text-primary font-medium" : "text-muted-foreground"
                      }`}
                    >
                      {getStepLabel(stepNumber)}
                    </span>
                  </div>
                  {i < totalSteps - 1 && (
                    <div
                      className={`w-8 h-0.5 mx-2 self-start mt-4 transition-colors ${
                        stepNumber < step ? "bg-primary" : "bg-muted"
                      }`}
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}
