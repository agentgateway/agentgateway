import styled from "@emotion/styled";
import { Card } from "antd";

export const StatisticCard = styled(Card)`
  flex: 1;
  border-color: var(--color-border-secondary);
  
  .ant-card-body {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: var(--spacing-lg);
  }
`;

export const StatisticContent = styled.div`
  display: flex;
  flex-direction: column;
  text-align: left;
`;

export const StatisticCardTitle = styled.h3`
  color: var(--color-text-secondary);
  font-size: var(--font-size-sm);
  margin: 0;
`

export const StatisticCardValue = styled.div`
  color: var(--color-text-base);
  font-size: 24px;
  font-weight: 600;
  margin: 0;
`;

export const StatisticCardIcon = styled.div`
  display: flex;
  align-items: center;
  justify-content: center;
  
  width: 50px;
  height: 50px;

  border-radius: 50%;
  box-shadow: 0px 1px 8px 0px rgba(255,255,255,0.2);
  background: var(--color-bg-spotlight);
  color: var(--color-text-base);
`;