import { CircleSlash, Send, TriangleAlert } from "lucide-react";
import { mockLLMErrorRateData, mockLLMLatencyData, mockPerModelLatencyDatasets, mockPerModelLatencyLabels, mockPerModelThroughputDatasets, mockPerModelThroughputLabels, mockRequestThroughputDataset, mockRequestThroughputLabels, mockTokenUsageByModelData } from "../../api/mockMetrics";
import { BarChart } from "../../components/Charts/BarChart";
import { HorizontalBarChart } from "../../components/Charts/HorizontalBarChart";
import { LineChart } from "../../components/Charts/LineChart";
import { Container } from "../../components/Layout/Container";
import { Row } from "../../components/Layout/Row";
import { Title } from "../../components/Layout/Title";
import { StatisticCard, StatisticCardIcon, StatisticCardTitle, StatisticCardValue, StatisticContent } from "../../components/StatisticCard/StatisticCard";
import { TimePickerSection } from "../../components/TimePickerSection/TimePickerSection";

export const LLMMetricsPage = () => (
  <Container>
    <TimePickerSection />
    <Row>
      <StatisticCard>
        <StatisticCardIcon>
          <TriangleAlert size={28} />
        </StatisticCardIcon>
        <StatisticContent>
          <StatisticCardTitle>Global Error Rate</StatisticCardTitle>
          <StatisticCardValue>0%</StatisticCardValue>
        </StatisticContent>
      </StatisticCard>
      <StatisticCard>
        <StatisticCardIcon>
          <Send size={28} />
        </StatisticCardIcon>
        <StatisticContent>
          <StatisticCardTitle>Total Requests</StatisticCardTitle>
          <StatisticCardValue>7</StatisticCardValue>
        </StatisticContent>
      </StatisticCard>
      <StatisticCard>
        <StatisticCardIcon>
          <CircleSlash size={28} />
        </StatisticCardIcon>
        <StatisticContent>
          <StatisticCardTitle>Tokens Used</StatisticCardTitle>
          <StatisticCardValue>406</StatisticCardValue>
        </StatisticContent>
      </StatisticCard>
    </Row>
    <div>
      <HorizontalBarChart 
        data={mockTokenUsageByModelData}
        title="Token Usage By Model"
      />
    </div>
    <Row>
      <LineChart 
        title="Request Throughput"
        labels={mockRequestThroughputLabels}
        datasets={mockRequestThroughputDataset}
      />
      <LineChart 
        title="Error Rates"
        labels={mockLLMErrorRateData.labels}
        datasets={mockLLMErrorRateData.datasets}
      />
    </Row>
    <Row>
      <BarChart 
        title="Latency Percentiles"
        labels={mockLLMLatencyData.labels}
        datasets={mockLLMLatencyData.datasets}
      />
    </Row>
    <Title>Per-Model Analytics</Title>
    <Row>
      <BarChart
        title="Avg Latency by Model (ms)"
        labels={mockPerModelLatencyLabels}
        datasets={mockPerModelLatencyDatasets}
      />
      <LineChart
        title="Request Volume by Model"
        labels={mockPerModelThroughputLabels}
        datasets={mockPerModelThroughputDatasets}
      />
    </Row>
  </Container>
);