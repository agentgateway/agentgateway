import { Alert, Button, Input, Spin, Typography } from "antd";
import { Check, CheckCircle } from "lucide-react";
import styled from "@emotion/styled";
import { useCallback, useState } from "react";
import {
  Actions,
  CopyableCommand,
  StepContent,
  StepDescription,
  StepList,
  StepNumber,
  StepRow,
  StepTitle,
} from "../../../components/wizard/WizardPrimitives";
import { useLLMWizard } from "./LLMWizardContext";

const { Link } = Typography;

const StyledInput = styled(Input)`
  width: 100%;
  border: 1px solid #d9d9d9 !important;
`;

const VerifyRow = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  margin-bottom: var(--spacing-lg);
`;

const SuccessText = styled.span`
  color: var(--color-success, #52c41a);
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  font-size: 14px;
  font-weight: 500;
`;

function OllamaWalkthrough() {
  const { data, setWalkthroughVerified, updateModelFields } = useLLMWizard();
  const [isVerifying, setIsVerifying] = useState(false);

  const handleVerify = async () => {
    setIsVerifying(true);
    setWalkthroughVerified(false, null);

    try {
      const res = await fetch(`http://${data.modelFields.hostOverride}/api/version`);
      if (res.ok) {
        setWalkthroughVerified(true, null);
      } else {
        setWalkthroughVerified(false, `Ollama responded with status ${res.status}`);
      }
    } catch {
      setWalkthroughVerified(false, "Could not reach Ollama at localhost:11434. Is it running?");
    } finally {
      setIsVerifying(false);
    }
  };

  return (
    <>
      <StepList>
        <StepRow>
          <StepNumber>1</StepNumber>
          <StepContent>
            <span>
              Pull a model from the{" "}
              <Link href="https://ollama.com/search" target="_blank" rel="noopener noreferrer">
                Ollama registry
              </Link>
            </span>
            <StyledInput
              value={data.modelFields.model}
              onChange={(e) => updateModelFields({ model: e.target.value })}
              placeholder="e.g. smallthinker"
            />
            <CopyableCommand>{`ollama pull ${data.modelFields.model}`}</CopyableCommand>
          </StepContent>
        </StepRow>

        <StepRow>
          <StepNumber>2</StepNumber>
          <StepContent>
            Start the server
            <CopyableCommand>ollama serve</CopyableCommand>
          </StepContent>
        </StepRow>

        <StepRow>
          <StepNumber>3</StepNumber>
          <StepContent>
            Set the Ollama target host
            <StyledInput
              value={data.modelFields.hostOverride}
              onChange={(e) => updateModelFields({ hostOverride: e.target.value })}
              placeholder="e.g. localhost:11434"
            />
          </StepContent>
        </StepRow>
      </StepList>

      <VerifyRow>
        <Button type="primary" ghost onClick={handleVerify} disabled={isVerifying}>
          {isVerifying ? <Spin size="small" /> : "Verify Ollama Connection"}
        </Button>
        {data.setupVerified && (
          <SuccessText>
            <CheckCircle size={16} /> Ollama detected
          </SuccessText>
        )}
      </VerifyRow>

      {data.setupVerifyError && (
        <Alert
          type="error"
          message={data.setupVerifyError}
          style={{ marginBottom: "var(--spacing-lg)" }}
        />
      )}
    </>
  );
}

const WALKTHROUGH_CONTENT: Record<string, React.ReactNode> = {
  ollama: <OllamaWalkthrough />,
};

export function SetupStep() {
  const { data, nextStep, previousStep } = useLLMWizard();
  const { selectedWalkthrough, setupVerified: walkthroughVerified } = data;

  const content = selectedWalkthrough
    ? WALKTHROUGH_CONTENT[selectedWalkthrough] ?? (
        <Typography.Text type="secondary">
          No walkthrough available for "{selectedWalkthrough}".
        </Typography.Text>
      )
    : null;

  return (
    <div>
      <StepTitle>Set your model provider</StepTitle>
      <StepDescription>
        Follow the steps below, then verify the connection before continuing.
      </StepDescription>

      {content}

      <Actions>
        <Button onClick={previousStep}>Back</Button>
        <Button type="primary" disabled={!walkthroughVerified} onClick={nextStep}>
          Next
        </Button>
      </Actions>
    </div>
  );
}
