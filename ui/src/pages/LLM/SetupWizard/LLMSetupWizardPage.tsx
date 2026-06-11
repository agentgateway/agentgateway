import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { useXdsMode } from "../../../api";
import { WizardShell } from "../../../components/wizard/WizardShell";
import { InstallStep } from "./InstallStep";
import { LLMWizardProvider, useLLMWizard } from "./LLMWizardContext";
import { ModelConfigStep } from "./ModelConfigStep";
import { SelectModelStep } from "./SelectModelStep";

const STEP_LABELS = [
  { title: "Model Type" },
  { title: "Install" },
  { title: "Configure" },
];

function LLMSetupWizardInner() {
  const { currentStep, stepIndex } = useLLMWizard();

  const stepComponent = {
    selectModel: <SelectModelStep />,
    install: <InstallStep />,
    modelConfig: <ModelConfigStep />,
  }[currentStep];

  return (
    <WizardShell title="LLM Setup Wizard" stepLabels={STEP_LABELS} stepIndex={stepIndex}>
      {stepComponent}
    </WizardShell>
  );
}

export function LLMSetupWizardPage() {
  const navigate = useNavigate();
  const { xdsMode } = useXdsMode();
  useEffect(() => {
    if (xdsMode) {
      navigate("/dashboard", { replace: true });
    }
  }, []);
  return (
    <LLMWizardProvider>
      <LLMSetupWizardInner />
    </LLMWizardProvider>
  );
}
