import { Button } from "antd";
import { Bot } from "lucide-react";
import { useNavigate } from "react-router-dom";
import {
  Actions,
  CardGrid,
  CardLabel,
  CardSubtext,
  OptionCard,
  StepTitle,
} from "../../../components/wizard/WizardPrimitives";
import { useLLMWizard } from "./LLMWizardContext";

const OPTIONS = [
  {
    id: "ollama",
    label: "Ollama",
    subtext: "Run models locally with Ollama",
  },
  {
    id: "manual",
    label: "Manual Configuration",
    subtext: "Set up your model manually",
  },
];

export function SelectModelStep() {
  const { data, setSelectedWalkthrough, nextStep, previousStep } = useLLMWizard();
  const selected = data.selectedWalkthrough;
  const navigate = useNavigate();

  const handleSelect = (id: string) => {
    setSelectedWalkthrough(id);
  };

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
          <Bot size={20} />
          How do you want to set up your model?
        </div>
      </StepTitle>
      <CardGrid>
        {OPTIONS.map(({ id, label, subtext }) => (
          <OptionCard
            key={id}
            $selected={selected === id}
            onClick={() => handleSelect(id)}
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
