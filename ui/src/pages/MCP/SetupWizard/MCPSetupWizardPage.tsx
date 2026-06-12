import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { useXdsMode } from "../../../api";
import { WizardShell } from "../../../components/wizard/WizardShell";
import { MCPWizardProvider, useMCPWizard } from "./MCPWizardContext";
import { SelectServerStep } from "./SelectServerStep";
import { ServerConfigStep } from "./ServerConfigStep";

const STEP_LABELS = [
  { title: "Server" },
  // { title: "Install" },  // streamableHttp only
  { title: "Configure" },
];

function MCPSetupWizardInner() {
  const { currentStep, stepIndex } = useMCPWizard();

  const stepComponent = {
    selectServer: <SelectServerStep />,
    // install: <InstallStep />,  // streamableHttp only
    config: <ServerConfigStep />,
  }[currentStep];

  return (
    <WizardShell title="MCP Setup Wizard" stepLabels={STEP_LABELS} stepIndex={stepIndex}>
      {stepComponent}
    </WizardShell>
  );
}

export function MCPSetupWizardPage() {
  const navigate = useNavigate();
  const { xdsMode } = useXdsMode();
  useEffect(() => {
    if (xdsMode) {
      navigate("/dashboard", { replace: true });
    }
  }, []);
  return (
    <MCPWizardProvider>
      <MCPSetupWizardInner />
    </MCPWizardProvider>
  );
}
