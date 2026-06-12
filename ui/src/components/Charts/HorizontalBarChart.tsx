import styled from "@emotion/styled";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: var(--spacing-lg);
  background-color: var(--color-bg-container);
  border: 1px solid var(--color-border-secondary);
  border-radius: var(--border-radius-md);
`;

const Header = styled.div`
  display: flex;
  justify-content: space-between;
  align-items: center;
`;

const Title = styled.h3`
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: var(--color-text-base);
`;

const Legend = styled.div`
  display: flex;
  gap: 16px;
  flex-wrap: wrap;
`;

const LegendItem = styled.div`
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  color: var(--color-text-secondary);
`;

const LegendColor = styled.div<{ color: string }>`
  width: 12px;
  height: 12px;
  border-radius: 2px;
  background-color: ${props => props.color};
`;

const BarContainer = styled.div`
  display: flex;
  gap: 5px;
  align-items: flex-end;
`;

const SegmentWrapper = styled.div<{ gap: string; proportion: string }>`
  display: flex;
  flex-direction: column;
  gap: ${props => props.gap || "12px"};
  flex: ${props => props.proportion};
  min-width: 0;
`;

const Segment = styled.div<{ color: string; height: string; borderRadius: string }>`
  position: relative;
  background-color: ${props => props.color};
  height: ${props => props.height || "48px"};
  border-radius: ${props => props.borderRadius || "4px"};
  transition: opacity 0.2s ease;
  cursor: pointer;

  &::after {
    content: '';
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0);
    border-radius: inherit;
    transition: background 0.2s ease;
  }

  &:hover::after {
    background: rgba(0, 0, 0, 0.15);
  }

  &:hover .tooltip {
    opacity: 1;
    visibility: visible;
  }
`;

const Tooltip = styled.div`
  position: absolute;
  bottom: 100%;
  left: 50%;
  transform: translateX(-50%);
  background: var(--color-bg-base);
  background-color: var(--color-bg-base);
  border: 1px solid var(--color-border-base);
  border-radius: 4px;
  padding: 12px;
  margin-bottom: 8px;
  white-space: nowrap;
  font-size: 12px;
  opacity: 0;
  visibility: hidden;
  transition: opacity 0.2s ease;
  z-index: 10;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
  backdrop-filter: none;

  &.visible {
    opacity: 1;
    visibility: visible;
  }
`;

const TooltipRow = styled.div`
  display: flex;
  justify-content: space-between;
  gap: 16px;
`;

const TooltipLabel = styled.span`
  font-weight: 600;
  color: var(--color-text-base);
`;

const TooltipDivider = styled.div`
  height: 1px;
  background-color: var(--color-border-base);
  margin: 8px 0;
`;

const Label = styled.div`
  font-size: 12px;
  color: var(--color-text-secondary);
  text-align: start;
  word-break: break-word;
`;

interface HorizontalBarChartData {
  label: string;
  value: number;
  color: string;
  tooltipData?: Array<{
    title: string;
    rows: Array<{ label?: string; value: string | number }>;
  }>;
}

interface HorizontalBarChartProps {
  data: HorizontalBarChartData[];
  title?: string;
  height?: string;
  borderRadius?: string;
  gap?: string;
}

export const HorizontalBarChart = ({
  data,
  title = "Horizontal Bar Chart",
  height = "48px",
  borderRadius = "4px",
  gap = "12px",
}: HorizontalBarChartProps) => {
  const total = data.reduce((sum, item) => sum + item.value, 0);

  if (total === 0) {
    return <div>No data available</div>;
  }

  return (
    <Container>
      <Header>
        <Title>{title}</Title>
        <Legend>
          {data.map((item) => (
            <LegendItem key={item.label}>
              <LegendColor color={item.color} />
              <span>{item.label}</span>
            </LegendItem>
          ))}
        </Legend>
      </Header>

      <BarContainer>
        {data.map((item) => {
          const proportion = (item.value / total) * 100;

          return (
            <SegmentWrapper
              key={item.label}
              proportion={`${proportion}%`}
              gap={gap}
            >
              <Segment
                color={item.color}
                height={height}
                borderRadius={borderRadius}
              >
                {item.tooltipData && (
                  <Tooltip className="tooltip">
                    {item.tooltipData.map((section, idx) => (
                      <div key={idx}>
                        <TooltipRow>
                          <TooltipLabel>{section.title}</TooltipLabel>
                        </TooltipRow>
                        {section.rows.map((row, ridx) => (
                          <TooltipRow key={ridx}>
                            <span style={{ fontSize: "11px" }}>
                              {row.label ? `${row.label}: ${row.value}` : row.value}
                            </span>
                          </TooltipRow>
                        ))}
                        {idx < (item.tooltipData?.length || 0) - 1 && <TooltipDivider />}
                      </div>
                    ))}
                  </Tooltip>
                )}
              </Segment>
              <Label>
                {item.label}: {item.value}
              </Label>
            </SegmentWrapper>
          );
        })}
      </BarContainer>
    </Container>
  );
};
