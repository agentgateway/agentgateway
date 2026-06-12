import { Button } from "antd";
import { Server } from "lucide-react";
import { useNavigate } from "react-router-dom";
import {
  Actions,
  CardGrid,
  CardLabel,
  CardSubtext,
  OptionCard,
  StepTitle,
} from "../../../components/wizard/WizardPrimitives";
import { useMCPWizard } from "./MCPWizardContext";

const OPTIONS = [
  // streamableHttp option — commented out, may be re-enabled by product
  // {
  //     id: "server-everything",
  //     label: "server-everything (streamableHttp)",
  //     subtext: "Run a local MCP server",
  // },
  {
    id: "server-everything-stdio",
    label: "stdio MCP Server",
    subtext: "Run a local MCP server using stdio",
  },
  {
    id: "manual",
    label: "Manual Configuration",
    subtext: "Set up your MCP server manually",
  },
];

export function SelectServerStep() {
  const { data, setSelectedServer, nextStep, previousStep } = useMCPWizard();
  const navigate = useNavigate();
  const selected = data.selectedServer;

  const handleNext = () => {
    if (!selected) return;
    if (selected === "manual") {
      navigate("/traffic-configuration");
      return;
    }
    nextStep();
  };

  return (
    <div>
      <StepTitle>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Server size={20} />
          How do you want to set up your MCP server?
        </div>
      </StepTitle>
      <CardGrid>
        {OPTIONS.map(({ id, label, subtext }) => (
          <OptionCard
            key={id}
            $selected={selected === id}
            onClick={() => setSelectedServer(id)}
          >
            <CardLabel $selected={selected === id}>{label}</CardLabel>
            <CardSubtext>{subtext}</CardSubtext>
          </OptionCard>
        ))}
      </CardGrid>
      <Actions>
        <Button onClick={previousStep}>Back</Button>
        <Button type="primary" disabled={!selected} onClick={handleNext}>
          Next
        </Button>
      </Actions>
    </div>
  );
}
