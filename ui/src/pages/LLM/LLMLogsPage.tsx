import styled from "@emotion/styled";
import { Card, Tag } from "antd";
import { FileText } from "lucide-react";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
`;

const PageTitle = styled.h1`
  margin: 0;
  font-size: 24px;
  font-weight: 600;
`;

const EmptyStateCard = styled(Card)`
  text-align: center;
  .ant-card-body {
    padding: 64px 32px;
  }
`;

const EmptyIcon = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  width: 64px;
  height: 64px;
  border-radius: 16px;
  background: var(--color-bg-hover);
  color: var(--color-text-tertiary);
  margin: 0 auto 16px;
`;

export const LLMLogsPage = () => (
  <Container>
    <PageTitle>LLM Logs</PageTitle>
    <EmptyStateCard>
      <EmptyIcon>
        <FileText size={28} />
      </EmptyIcon>
      <h3 style={{ margin: "0 0 8px", fontSize: 18, fontWeight: 600 }}>
        LLM Request Logs
      </h3>
      <p
        style={{
          margin: "0 0 24px",
          color: "var(--color-text-secondary)",
          maxWidth: 400,
          marginLeft: "auto",
          marginRight: "auto",
        }}
      >
        LLM request and response logs will appear here, including token usage,
        model selections, latency, and guardrail decisions.
      </p>
      <Tag bordered={false} color="processing" style={{ padding: "4px 12px", fontSize: 13 }}>
        Coming soon
      </Tag>
    </EmptyStateCard>
  </Container>
);
