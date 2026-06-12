import styled from "@emotion/styled";
import { useEffect, useState } from "react";
import { MOCK_TRAFFIC_LOGS } from "../../api/mockLogs";
import { StreamingLogViewer } from "../../components/StreamingLogViewer/StreamingLogViewer";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  height: 100%;
  overflow: hidden;
`;

export const TrafficLogsPage = () => {
  const [mockTrafficLogs, setMockTrafficLogs] = useState<string[]>([]);

  useEffect(() => { 
    let index = 0;
    const interval = setInterval(() => {
      if (index < MOCK_TRAFFIC_LOGS.length) { 
        setMockTrafficLogs(prev => [...prev, MOCK_TRAFFIC_LOGS[index]]);
        index++;
      } else { 
        clearInterval(interval);
      }
    }, 500);

    return () => clearInterval(interval);
  }, [])

  return (
    <Container>
      <StreamingLogViewer 
        data={mockTrafficLogs}
      />
    </Container>
  );
};