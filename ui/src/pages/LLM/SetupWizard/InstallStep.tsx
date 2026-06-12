import styled from "@emotion/styled";
import { Button, Tabs, Typography } from "antd";
import { Download } from "lucide-react";
import { LinuxLogo, MacLogo, WindowsLogo } from "../../../assets/logos";
import { detectPlatform } from "../../../utils/platform";
import {
  Actions,
  CopyableCommand,
  StepContent,
  StepList,
  StepRow,
  StepTitle,
} from "../../../components/wizard/WizardPrimitives";
import { useLLMWizard } from "./LLMWizardContext";

const { Link } = Typography;

const StyledTabs = styled(Tabs)`
  display: flex;
  align-items: center;
  .ant-tabs-tab {
    color: var(--color-text-secondary);
    width: 100px !important;
    justify-content: center;
    background-color: transparent !important;
    border-bottom: none !important;
    padding: 5px 0 !important;
  }
  .ant-tabs-tab-active .ant-tabs-tab-btn {
    color: var(--color-text-base) !important;
  }
  .ant-tabs-tab-active {
    background-color: #f0f0f0 !important;
  }
`;

const TabContainer = styled.div`
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--spacing-xs);
`;

function OllamaInstall() {
  const platform = detectPlatform();

  return (
    <StepList>
      <StepRow>
        <StepContent>
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--spacing-md)" }}>
            <StyledTabs
              type="card"
              size="small"
              defaultActiveKey={platform}
              items={[
                {
                  key: "macos",
                  label: (
                    <TabContainer>
                      <MacLogo />
                      macOS
                    </TabContainer>
                  ),
                  children: <CopyableCommand>curl -fsSL https://ollama.com/install.sh | sh</CopyableCommand>,
                },
                {
                  key: "linux",
                  label: (
                    <TabContainer>
                      <LinuxLogo />
                      Linux
                    </TabContainer>
                  ),
                  children: <CopyableCommand>curl -fsSL https://ollama.com/install.sh | sh</CopyableCommand>,
                },
                {
                  key: "windows",
                  label: (
                    <TabContainer>
                      <WindowsLogo />
                      Windows
                    </TabContainer>
                  ),
                  children: <CopyableCommand>irm https://ollama.com/install.ps1 | iex</CopyableCommand>,
                },
              ]}
            />
            <span>
              Or, download and install Ollama from the official{" "}
              <Link href="https://ollama.com/download" target="_blank" rel="noopener noreferrer">
                ollama.com/download
              </Link>{" "}
              website.
            </span>
          </div>
        </StepContent>
      </StepRow>
    </StepList>
  );
}

const WALKTHROUGH_CONTENT: Record<string, React.ReactNode> = {
  ollama: <OllamaInstall />,
};

export function InstallStep() {
  const { data, nextStep, previousStep } = useLLMWizard();
  const { selectedWalkthrough } = data;

  const content = selectedWalkthrough
    ? WALKTHROUGH_CONTENT[selectedWalkthrough] ?? (
        <Typography.Text type="secondary">
          No walkthrough available for "{selectedWalkthrough}".
        </Typography.Text>
      )
    : null;

  return (
    <div>
      <StepTitle>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Download size={20} />
          Install Ollama
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
