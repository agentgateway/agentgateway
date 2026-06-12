import { Button } from "antd";
import { Download } from "lucide-react";
import { Actions, StepTitle } from "../../../components/wizard/WizardPrimitives";
import { useMCPWizard } from "./MCPWizardContext";

// streamableHttp only — uncomment when re-enabling server-everything
// function ServerEverythingInstall() { ... }

// streamableHttp only — uncomment when re-enabling server-everything
// const PROVIDER_CONTENT: Record<string, React.ReactNode> = {
//     "server-everything": <ServerEverythingInstall />,
// };

export function InstallStep() {
  const { nextStep, previousStep } = useMCPWizard();
  // const content = PROVIDER_CONTENT[data.selectedServer ?? ""] ?? null;  // streamableHttp only
  const content = null;

  return (
    <div>
      <StepTitle>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Download size={20} />
          Install MCP server
        </div>
      </StepTitle>
      {content}
      <Actions>
        <Button onClick={previousStep}>Back</Button>
        <Button type="primary" onClick={nextStep}>
          Next
        </Button>
      </Actions>
    </div>
  );
}
