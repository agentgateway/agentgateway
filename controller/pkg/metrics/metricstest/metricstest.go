// Package metricstest provides utilities for testing metrics.
package metricstest

import (
	"context"
	"fmt"
	"io"

	"github.com/prometheus/client_golang/prometheus/testutil"
	"github.com/prometheus/client_golang/prometheus/testutil/promlint"
	dto "github.com/prometheus/client_model/go"
	"github.com/prometheus/common/expfmt"
	"github.com/prometheus/common/model"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	"github.com/kgateway-dev/kgateway/v2/pkg/metrics"
)

// HistogramMetricOutput is a struct to hold histogram metric output values.
type HistogramMetricOutput struct {
	SampleCount uint64
	SampleSum   float64
}

type ExpectMetric interface {
	GetLabels() []metrics.Label
	Match(t require.TestingT, value float64) bool
}

// ExpectedMetric is a struct to hold a metric label and value.
type ExpectedMetric struct {
	Labels []metrics.Label
	Value  float64
}

func (m *ExpectedMetric) GetLabels() []metrics.Label {
	return m.Labels
}

func (m *ExpectedMetric) Match(t require.TestingT, value float64) bool {
	return m.Value == value
}

var _ ExpectMetric = &ExpectedMetric{}

// ExpectedMetricValueTest is a struct to hold a metric label and a test function to match the value.
type ExpectedMetricValueTest struct {
	Labels []metrics.Label
	Test   func(value float64) bool
}

func (m *ExpectedMetricValueTest) Match(t require.TestingT, value float64) bool {
	return m.Test(value)
}

func (m *ExpectedMetricValueTest) GetLabels() []metrics.Label {
	return m.Labels
}

// Between returns a function that checks if a value is between or equal to a minimum and maximum value.
func Between(minVal, maxVal float64) func(value float64) bool {
	return func(value float64) bool {
		return value >= minVal && value <= maxVal
	}
}

// Equal returns a function that checks if a value is equal to another value.
func Equal(val float64) func(value float64) bool {
	return func(value float64) bool {
		return value == val
	}
}

// GreaterOrEqual returns a function that checks if a value is greater than or equal to another value
func GreaterOrEqual(val float64) func(value float64) bool {
	return func(value float64) bool {
		return value >= val
	}
}

var _ ExpectMetric = &ExpectedMetricValueTest{}

// Gathered metrics interface.
type GatheredMetrics interface {
	AssertMetricLabels(name string, expectedLabels []metrics.Label)
	AssertMetricsLabels(name string, expectedLabels [][]metrics.Label)
	AssertMetricsLabelsInclude(name string, expectedLabels [][]metrics.Label)
	AssertMetricHistogramValue(name string, expectedValue HistogramMetricOutput)
	AssertHistogramPopulated(name string)
	AssertHistogramBuckets(name string, expectedBuckets []float64)
	AssertMetricExists(name string)
	AssertMetricNotExists(name string)
	AssertMetric(name string, expectedMetric ExpectMetric)
	AssertMetrics(name string, expectedMetrics []ExpectMetric)
	AssertMetricsInclude(name string, expectedMetrics []ExpectMetric)
	Length() int
	MetricLength(name string) int
}

// MustGatherMetrics gathers metrics and returns them as GatheredMetrics.
func MustGatherMetrics(t require.TestingT) GatheredMetrics {
	return MustGatherPrometheusMetrics(t)
}

// Gathered metrics implementation for prometheus metrics.
type prometheusGatheredMetrics struct {
	metrics map[string][]*dto.Metric
	t       require.TestingT
}

var _ GatheredMetrics = &prometheusGatheredMetrics{}

// Length returns the number of different metric names gathered.
func (g *prometheusGatheredMetrics) Length() int {
	return len(g.metrics)
}

// MetricLength returns the number of metrics for a given name.
func (g *prometheusGatheredMetrics) MetricLength(name string) int {
	if metrics, ok := g.metrics[name]; ok {
		return len(metrics)
	}

	return 0
}

// MustGatherMetricsContext gathers metrics and returns them as GatheredMetrics.
// It will block and attempt to gather metrics until either the context is done,
// an attempt to gather metrics results in an error, or the metrics are gathered.
// If the context completes before metrics are gathered, it will fail the test.
// If no names are provided, it will return upon gathering any metrics.
// If a histogram metric is included amongst the names it will attempt to gather
// until the histogram contains at least one sample.
func MustGatherMetricsContext(ctx context.Context, t require.TestingT, names ...string) GatheredMetrics {
Loop:
	for {
		select {
		case <-ctx.Done():
			require.Fail(t, "Context done before gathering metrics", "names", names)

			return nil
		default:
		}

		gathered := MustGatherPrometheusMetrics(t)

		if gathered == nil || gathered.Length() == 0 {
			continue Loop
		}

		for _, name := range names {
			if gathered.MetricLength(name) == 0 {
				continue Loop
			}

			if g, ok := gathered.(*prometheusGatheredMetrics); ok {
				metric := g.getFirstMetric(name)
				if metric == nil {
					continue Loop
				}

				if h := metric.GetHistogram(); h != nil && h.GetSampleCount() == 0 {
					continue Loop
				}
			}
		}

		return gathered
	}
}

// MustGatherPrometheusMetrics gathers metrics from the registry and returns them.
func MustGatherPrometheusMetrics(t require.TestingT) GatheredMetrics {
	gathered := prometheusGatheredMetrics{
		metrics: make(map[string][]*dto.Metric),
		t:       t,
	}
	metricFamilies, err := metrics.Registry().Gather()
	require.NoError(t, err)

	for _, mf := range metricFamilies {
		metrics := make([]*dto.Metric, len(mf.GetMetric()))
		copy(metrics, mf.GetMetric())
		gathered.metrics[mf.GetName()] = metrics
	}

	return &gathered
}

// MustParseGatheredMetrics parses gathered metrics from the provided data reader.
func MustParseGatheredMetrics(t require.TestingT, data io.Reader) GatheredMetrics {
	gathered := prometheusGatheredMetrics{
		metrics: make(map[string][]*dto.Metric),
		t:       t,
	}

	parser := expfmt.NewTextParser(model.LegacyValidation)

	metricFamilies, err := parser.TextToMetricFamilies(data)
	require.NoError(t, err)

	for _, mf := range metricFamilies {
		metrics := make([]*dto.Metric, len(mf.GetMetric()))
		copy(metrics, mf.GetMetric())
		gathered.metrics[mf.GetName()] = metrics
	}

	return &gathered
}

// mustGetMetric retrieves a single metric by name, ensuring it exists and has exactly one instance.
func (g *prometheusGatheredMetrics) mustGetMetric(name string) *dto.Metric {
	m, ok := g.metrics[name]
	require.True(g.t, ok, "Metric %s not found", name)
	require.Equal(g.t, 1, len(m), "Expected 1 metric for %s", name)
	return m[0]
}

// getFirstMetric retrieves the first instance of a single metric by name, if it exists.
func (g *prometheusGatheredMetrics) getFirstMetric(name string) *dto.Metric {
	if m, ok := g.metrics[name]; ok && len(m) > 0 {
		return m[0]
	}

	return nil
}

// mustGetMetrics retrieves multiple metrics by name, ensuring they exist and have at least the expected count.
func (g *prometheusGatheredMetrics) mustGetMetrics(name string, expectedCount int) []*dto.Metric {
	m, ok := g.metrics[name]
	require.True(g.t, ok, "Metric %s not found", name)
	require.LessOrEqual(g.t, expectedCount, len(m), "Expected %d metrics for %s", expectedCount, name)
	return m
}

// assertMetricObjLabels asserts that a metric has the expected labels.
func (g *prometheusGatheredMetrics) assertMetricObjLabels(metric *dto.Metric, expectedLabels []metrics.Label) {
	err := g.metricObjLabelsMatch(metric, expectedLabels)
	assert.NoError(g.t, err)
}

func (g *prometheusGatheredMetrics) metricObjLabelsMatch(metric *dto.Metric, expectedLabels []metrics.Label) error {
	if len(expectedLabels) != len(metric.GetLabel()) {
		return fmt.Errorf("expected %d labels, got %d", len(expectedLabels), len(metric.GetLabel()))
	}

	labelMap := make(map[string]string, len(expectedLabels))

	for _, label := range expectedLabels {
		labelMap[label.Name] = label.Value
	}

	for _, label := range metric.GetLabel() {
		labelValue, ok := labelMap[label.GetName()]
		if !ok {
			return fmt.Errorf("label %s not found", label.GetName())
		}
		if labelValue != label.GetValue() {
			return fmt.Errorf("label %s value mismatch - expected %s, got %s", label.GetName(), labelValue, label.GetValue())
		}
	}

	return nil
}

// findMetricObj checks that the labels on a gathered metric match one of the expected sets of labels and returns the match
func (g *prometheusGatheredMetrics) findMetricObj(metric *dto.Metric, metricsToSearch []ExpectMetric) ExpectMetric {
	for _, m := range metricsToSearch {
		err := g.metricObjLabelsMatch(metric, m.GetLabels())
		if err == nil {
			return m
		}
	}

	return nil
}

// AssertMetricLabels asserts that a metric has the expected labels.
func (g *prometheusGatheredMetrics) AssertMetricLabels(name string, expectedLabels []metrics.Label) {
	metric := g.mustGetMetric(name)

	g.assertMetricObjLabels(metric, expectedLabels)
}

// AssertMetricsLabels asserts that multiple metrics of the same name exactly match the expected labels.
func (g *prometheusGatheredMetrics) AssertMetricsLabels(name string, expectedLabels [][]metrics.Label) {
	metrics := g.mustGetMetrics(name, len(expectedLabels))
	for i, m := range metrics {
		g.assertMetricObjLabels(m, expectedLabels[i])
	}
}

// AssertMetricsLabelsInclude asserts that multiple metrics of the same name include at least the expected labels.
func (g *prometheusGatheredMetrics) AssertMetricsLabelsInclude(name string, expectedLabels [][]metrics.Label) {
	metrics := g.mustGetMetrics(name, len(expectedLabels))

	matched := make([]bool, len(expectedLabels))

	for _, m := range metrics {
		for i, labels := range expectedLabels {
			if matched[i] {
				continue
			}

			err := g.metricObjLabelsMatch(m, labels)
			if err == nil {
				matched[i] = true

				break
			}
		}
	}

	allMatched := true

	for _, v := range matched {
		if !v {
			allMatched = false

			break
		}
	}

	if !allMatched {
		assert.Fail(g.t, "Not all expected labels were found within the metrics",
			"Expected metrics with labels %v for %s: %v",
			expectedLabels, name, metrics)
	}
}

// AssertMetricHistogramValue asserts that a histogram metric has the expected sample count and sum.
func (g *prometheusGatheredMetrics) AssertMetricHistogramValue(name string, expectedValue HistogramMetricOutput) {
	metric := g.mustGetMetric(name)
	assert.Equal(g.t, expectedValue, HistogramMetricOutput{
		SampleCount: metric.GetHistogram().GetSampleCount(),
		SampleSum:   metric.GetHistogram().GetSampleSum(),
	}, "Metric %s histogram value mismatch - expected %v, got %v", name, expectedValue, HistogramMetricOutput{
		SampleCount: metric.GetHistogram().GetSampleCount(),
		SampleSum:   metric.GetHistogram().GetSampleSum(),
	})
}

// AssertHistogramPopulated asserts that a histogram metric is populated
// (any histogram metric that has a non-zero sample count).
func (g *prometheusGatheredMetrics) AssertHistogramPopulated(name string) {
	metric := g.getFirstMetric(name)
	assert.NotNil(g.t, metric, "Metric %s not found", name)
	assert.NotNil(g.t, metric.GetHistogram(), "Metric %s is not a histogram", name)
	assert.True(g.t, metric.GetHistogram().GetSampleCount() > 0, "Histogram %s is not populated", name)
}

// AssertHistogramBuckets asserts that a histogram metric has the expected bucket values.
func (g *prometheusGatheredMetrics) AssertHistogramBuckets(name string, expectedBuckets []float64) {
	metric := g.mustGetMetric(name)

	histogram := metric.GetHistogram()
	require.NotNil(g.t, histogram, "Metric %s is not a histogram", name)

	buckets := histogram.GetBucket()
	require.Equal(g.t, len(expectedBuckets), len(buckets), "Expected %d buckets for histogram %s, got %d", len(expectedBuckets), name, len(buckets))

	for i, bucket := range buckets {
		assert.Equal(g.t, expectedBuckets[i], bucket.GetUpperBound(), "Bucket %d for histogram %s does not match expected value", i, name)
	}
}

// AssertMetricExists asserts that a metric with the given name exists.
func (g *prometheusGatheredMetrics) AssertMetricExists(name string) {
	_, ok := g.metrics[name]
	assert.True(g.t, ok, "Metric %s not found", name)
}

// AssertMetricNotExists asserts that a metric with the given name does not exist.
func (g *prometheusGatheredMetrics) AssertMetricNotExists(name string) {
	_, ok := g.metrics[name]
	assert.False(g.t, ok, "Metric %s found", name)
}

// AssertMetric asserts that a metric with the given name matches the expected metric.
// Works for counters and gauges, but not histograms or summaries.
func (g *prometheusGatheredMetrics) AssertMetric(name string, expected ExpectMetric) {
	g.AssertMetrics(name, []ExpectMetric{expected})
}

// AssertMetrics asserts that all metrics with the given name exactly match the expected metrics.
func (g *prometheusGatheredMetrics) AssertMetrics(name string, expectedMetrics []ExpectMetric) {
	require.NotEmpty(g.t, g.metrics[name], "Expected metrics %s not found", name)

	for _, m := range g.metrics[name] {
		matchedExpectedMetric := g.findMetricObj(m, expectedMetrics)
		require.NotNil(g.t, matchedExpectedMetric, "Metric %s with labels %v not found", name, m.GetLabel())
		assert.True(g.t, matchedExpectedMetric.Match(g.t, g.mustGetMetricValue(m)), "Metric %s value mismatch -  value is %f", name, g.mustGetMetricValue(m))
	}
}

// AssertMetricsInclude asserts that all metrics with the given name include at least all of the expected metrics.
func (g *prometheusGatheredMetrics) AssertMetricsInclude(name string, expectedMetrics []ExpectMetric) {
	require.NotEmpty(g.t, g.metrics[name], "Expected metrics %s not found", name)

	for _, m := range g.metrics[name] {
		matchedExpectedMetric := g.findMetricObj(m, expectedMetrics)
		if matchedExpectedMetric == nil {
			continue
		}

		assert.True(g.t, matchedExpectedMetric.Match(g.t, g.mustGetMetricValue(m)), "Metric %s value mismatch -  value is %f", name, g.mustGetMetricValue(m))
	}
}

// Both counters and gauges are supported.
func (g *prometheusGatheredMetrics) mustGetMetricValue(metric *dto.Metric) float64 {
	switch {
	case metric.GetCounter() != nil:
		return metric.GetCounter().GetValue()
	case metric.GetGauge() != nil:
		return metric.GetGauge().GetValue()
	default:
		assert.Fail(g.t, "Metric is not a counter or gauge")
		return 0
	}
}

// GatherAndLint gathers metrics and runs a linter on them.
func GatherAndLint(metricNames ...string) ([]promlint.Problem, error) {
	return testutil.GatherAndLint(metrics.Registry(), metricNames...)
}

// GatherAndCompare gathers metrics and compares them against expected values.
func GatherAndCompare(expected io.Reader, metricNames ...string) error {
	return testutil.GatherAndCompare(metrics.Registry(), expected, metricNames...)
}

// CollectAndCompare collects metrics from a collector and compares them against expected values.
func CollectAndCompare(c any, expected io.Reader, metricNames ...string) error {
	if err := testutil.CollectAndCompare(metrics.GetPromCollector(c), expected, metricNames...); err != nil {
		return err
	}

	return nil
}
