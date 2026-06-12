import { mockRequestCountByRouteData, mockTrafficErrorRateDatasets, mockTrafficErrorRateLabels, mockTrafficLatencyDistributionDatasets, mockTrafficLatencyDistributionLabels, mockTrafficPerRouteLatencyDatasets, mockTrafficPerRouteLatencyLabels, mockTrafficPerRouteVolumeDatasets, mockTrafficPerRouteVolumeLabels } from "../../api/mockMetrics";
import { BarChart } from "../../components/Charts/BarChart";
import { HorizontalBarChart } from "../../components/Charts/HorizontalBarChart";
import { LineChart } from "../../components/Charts/LineChart";
import { Container } from "../../components/Layout/Container";
import { Row } from "../../components/Layout/Row";
import { Title } from "../../components/Layout/Title";
import { TimePickerSection } from "../../components/TimePickerSection/TimePickerSection";

/**
 * Request counts
 * latency distributions
 * error rates
 * per-route analytics
 */
export const TrafficMetricsPage = () => (
  <Container>
    <TimePickerSection />
    <HorizontalBarChart
      data={mockRequestCountByRouteData}
      title="Request Count by Route"
    />
    <Row>
      <BarChart 
        title="Latency Distribution"
        labels={mockTrafficLatencyDistributionLabels}
        datasets={mockTrafficLatencyDistributionDatasets}
      />
      <LineChart
        title="Error Rate"
        labels={mockTrafficErrorRateLabels}
        datasets={mockTrafficErrorRateDatasets}
      />
    </Row>
    <Title>Per-Route Analytics</Title>
    <Row>
      <BarChart
        title="Avg Latency by Route (ms)"
        labels={mockTrafficPerRouteLatencyLabels}
        datasets={mockTrafficPerRouteLatencyDatasets}
      />
      <LineChart 
        title="Request Volume by Route"
        labels={mockTrafficPerRouteVolumeLabels}
        datasets={mockTrafficPerRouteVolumeDatasets}
      />
    </Row>
  </Container>
);
