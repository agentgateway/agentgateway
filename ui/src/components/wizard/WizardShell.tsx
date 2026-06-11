import styled from "@emotion/styled";
import { Card, ConfigProvider, Steps } from "antd";
import type { ReactNode } from "react";

const PageRoot = styled.div`
  display: flex;
  flex-direction: column;
  overflow: hidden;
`;

const PageHeader = styled.div`
  padding: var(--spacing-lg) var(--spacing-xl);
  border-bottom: 1px solid var(--color-border);
  border: 1px solid var(--color-border-secondary);
  background: linear-gradient(to right, var(--color-bg-hover) 0%, var(--color-bg-container) 100%);
`;

const PageTitle = styled.h1`
  margin: 0 0 var(--spacing-md) 0;
  font-size: 24px;
  font-weight: 600;
  color: var(--color-text-base);
  border-left: 3px solid #6941c6;
  padding-left: var(--spacing-sm);
`;

const StepBody = styled.div`
  flex: 1;
  overflow-y: auto;
  display: flex;
  justify-content: center;
  padding: var(--spacing-xl);
`;

interface WizardShellProps {
  title: string;
  stepLabels: { title: string }[];
  stepIndex: number;
  children: ReactNode;
}

export function WizardShell({ title, stepLabels, stepIndex, children }: WizardShellProps) {
  return (
    <PageRoot>
      <PageHeader>
        <PageTitle>{title}</PageTitle>
      </PageHeader>
      <StepBody>
        <Card
          style={{
            width: "100%",
            maxWidth: 640,
            "--color-border-secondary": "rgba(0, 0, 0, 0.3)",
            boxShadow: "0px 4px 20px rgba(0, 0, 0, 0.3)",
          } as React.CSSProperties}
        >
          <ConfigProvider theme={{ token: { colorPrimary: "#6941c6" } }}>
            <Steps
              current={stepIndex}
              items={stepLabels}
              labelPlacement="vertical"
              size="small"
              style={{ marginBottom: "var(--spacing-xl)" }}
            />
          </ConfigProvider>
          {children}
        </Card>
      </StepBody>
    </PageRoot>
  );
}
