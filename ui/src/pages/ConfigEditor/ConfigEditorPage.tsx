import styled from "@emotion/styled";
import { useEffect, useMemo } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { ConfigEditor } from "./ConfigEditor";

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

export function ConfigEditorPage() {
  const navigate = useNavigate();
  const location = useLocation();

  const basePath = useMemo(
    () => location.pathname.replace(/\/editor.*$/, "") || "/",
    [location.pathname],
  );

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
            Edit the complete configuration in JSON or YAML format with schema validation
          </Description>
        </div>
      </PageHeader>
      <ConfigEditor onClose={() => navigate(basePath)} />
    </Container>
  );
}
