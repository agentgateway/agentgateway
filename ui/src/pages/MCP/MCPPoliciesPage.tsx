import styled from "@emotion/styled";
import { Card, Empty } from "antd";
import { Shield } from "lucide-react";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
`;

const PageTitle = styled.h1`
  margin: 0 0 4px;
  font-size: 24px;
  font-weight: 600;
`;

const PageSubtitle = styled.p`
  margin: 0;
  color: var(--color-text-secondary);
  font-size: 14px;
`;

export const MCPPoliciesPage = () => {
  return (
    <Container>
      <div>
        <PageTitle>MCP Policies</PageTitle>
        <PageSubtitle>
          Configure policies for MCP traffic including authentication, authorization, and rate limiting
        </PageSubtitle>
      </div>

      <Card>
        <Empty
          image={<Shield size={48} style={{ opacity: 0.3 }} />}
          description="MCP policies configuration coming soon"
        />
      </Card>
    </Container>
  );
};
