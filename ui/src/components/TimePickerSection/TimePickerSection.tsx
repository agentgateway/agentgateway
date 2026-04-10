import styled from "@emotion/styled";
import { Button, Segmented, Tooltip } from "antd";
import { Clock, RefreshCw } from "lucide-react";
import { useState } from "react";
import { CustomTimePickerDropdown } from "./CustomTimePickerDropdown";

const timeOptions = ['1m', '5m', '15m', '30m', '1h', '6h', '24h', '7d', '30d'];

const Container = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing-md);
`;

const Controls = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
`;

const CustomButtonWrapper = styled.div`
  position: relative;
`;

const StyledButton = styled(Button)`
  &.ant-btn-default { 
    background: var(--color-bg-base) !important;
    border-radius: 20px !important;
  }

  [data-theme="dark"] &.ant-btn-default {
    background: #1a1a1a !important;
  }
`

const StyledSegmented = styled(Segmented<string>)`
  &.ant-segmented {
    border-radius: 20px;
    padding: 2px;
    color: var(--color-text-base) !important;
  }
  .ant-segmented-item {
    border-radius: 18px;
  }
  .ant-segmented-thumb {
    border-radius: 18px !important;
  }
`

export const TimePickerSection = () => {
  const [activeTab, setActiveTab] = useState<string>('30d');
  const [isCustomActive, setIsCustomActive] = useState(false);
  const [customOpen, setCustomOpen] = useState(false);
  const [customStart, setCustomStart] = useState<Date | undefined>();
  const [customEnd, setCustomEnd] = useState<Date | undefined>();

  const handleTabChange = (value: string) => {
    setActiveTab(value);
    setIsCustomActive(false);
  };

  return (
    <Container>
      <Controls>
        <StyledSegmented
          options={timeOptions}
          value={isCustomActive ? 'custom' : activeTab}
          onChange={handleTabChange}
        />
        <CustomButtonWrapper>
          <StyledButton
            icon={<Clock size={14} />}
            type={isCustomActive ? 'primary' : 'default'}
            onClick={() => setCustomOpen(prev => !prev)}
          >
            Custom
          </StyledButton>
          <CustomTimePickerDropdown
            isOpen={customOpen}
            savedStart={customStart}
            savedEnd={customEnd}
            onClose={() => setCustomOpen(false)}
            onApply={(start, end) => {
              setCustomStart(start);
              setCustomEnd(end);
              setIsCustomActive(true);
              setCustomOpen(false);
            }}
          />
        </CustomButtonWrapper>
      </Controls>
      <Tooltip title="Refresh Data" placement="left">
        <StyledButton icon={<RefreshCw size={14} />} shape="circle" />
      </Tooltip>
    </Container>
  );
};
