import styled from "@emotion/styled";
import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { RawConfigEditor } from "./RawConfigEditor";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  height: 100%;
  overflow: hidden;
`;

const PageHeader = styled.div`
  display: flex;
  justify-content: space-between;
  align-items: center;
`;

const PageTitle = styled.h1`
  margin: 0;
  font-size: 24px;
  font-weight: 600;
  color: var(--color-text-base);
`;

const Description = styled.p`
  color: var(--color-text-secondary);
  margin: 0;
  font-size: 14px;
`;

export function RawConfigPage() {
  const navigate = useNavigate();

  useEffect(() => {
    document.title = "Config Editor - agentgateway";
    return () => {
      document.title = "agentgateway";
    };
  }, []);

  return (
    <Container>
      <PageHeader>
        <div>
          <PageTitle>Configuration Editor</PageTitle>
          <Description>
            Edit the complete configuration with JSON schema validation
          </Description>
        </div>
      </PageHeader>
      <RawConfigEditor onClose={() => navigate("/traffic")} />
    </Container>
  );
}
