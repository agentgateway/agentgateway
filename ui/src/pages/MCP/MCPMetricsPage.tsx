import { mockMCPErrorRateData, mockMCPLatencyDistributionDatasets, mockMCPLatencyDistributionLabels, mockPerTargetCallsDatasets, mockPerTargetCallsLabels, mockPerTargetLatencyDatasets, mockPerTargetLatencyLabels, mockToolCallCountsData } from "../../api/mockMetrics";
import { BarChart } from "../../components/Charts/BarChart";
import { HorizontalBarChart } from "../../components/Charts/HorizontalBarChart";
import { LineChart } from "../../components/Charts/LineChart";
import { Container } from "../../components/Layout/Container";
import { Row } from "../../components/Layout/Row";
import { Title } from "../../components/Layout/Title";
import { TimePickerSection } from "../../components/TimePickerSection/TimePickerSection";

export const MCPMetricsPage = () => (
  <Container>
    <TimePickerSection />
    <HorizontalBarChart 
      data={mockToolCallCountsData}
      title="Tool Call Counts"
    />
    <Row>
      <BarChart
        title="Latency Distributions"
        labels={mockMCPLatencyDistributionLabels}
        datasets={mockMCPLatencyDistributionDatasets}
      />
      <LineChart 
        title="Error Rates"
        labels={mockMCPErrorRateData.labels}
        datasets={mockMCPErrorRateData.datasets}
      />
    </Row>
    <Title>Per-Target Analytics</Title>
    <Row>
      <BarChart
        title="Avg Latency by Target (ms)"
        labels={mockPerTargetLatencyLabels}
        datasets={mockPerTargetLatencyDatasets}
      />
      <LineChart
        title="Tool Calls Over Time"
        labels={mockPerTargetCallsLabels}
        datasets={mockPerTargetCallsDatasets}
      />
    </Row>
  </Container>
);
