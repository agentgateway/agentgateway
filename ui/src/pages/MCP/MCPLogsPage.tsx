import styled from "@emotion/styled";
import { useEffect, useState } from "react";
import { MOCK_MCP_LOGS } from "../../api/mockLogs";
import { SoloLogViewer } from "../../components/LogViewer/SoloLogViewer";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  height: 100%;
  overflow: hidden;
`;

export const MCPLogsPage = () => {
  const [mockMCPLogs, setMockMCPLogs] = useState<string[]>([]);

  useEffect(() => { 
    let index = 0;
    const interval = setInterval(() => { 
      if (index < MOCK_MCP_LOGS.length) { 
        setMockMCPLogs(prev => [...prev, MOCK_MCP_LOGS[index]]);
        index++;
      } else {
        clearInterval(interval);
      }
    }, 500);

    return () => clearInterval(interval);
  }, [])

  return (
    <Container>
      <SoloLogViewer data={mockMCPLogs} />
    </Container>
  );
};
