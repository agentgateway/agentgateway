import styled from "@emotion/styled";
import { useEffect, useState } from "react";
import { MOCK_LLM_LOGS } from "../../api/mockLogs";
import { SoloLogViewer } from "../../components/LogViewer/SoloLogViewer";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  height: 100%;
  overflow: hidden;
`;

export const LLMLogsPage = () => {
  const [mockLLMLogs, setMockLLMLogs] = useState<string[]>([]);

  useEffect(() => { 
    let index = 0;
    const interval = setInterval(() => { 
      if (index < MOCK_LLM_LOGS.length) { 
        setMockLLMLogs(prev => [...prev, MOCK_LLM_LOGS[index]]);
        index++;
      } else {
        clearInterval(interval);
      }
    }, 500);

    return () => clearInterval(interval);
  }, [])

  return (
    <Container>
      <SoloLogViewer data={mockLLMLogs} />
    </Container>
  );
};
