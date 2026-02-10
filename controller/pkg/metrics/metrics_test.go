package metrics_test

import (
	"testing"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"

	. "github.com/kgateway-dev/kgateway/v2/pkg/metrics"
	"github.com/kgateway-dev/kgateway/v2/pkg/metrics/metricstest"
)

func setupTestRegistry() {
	SetRegistry(false, NewRegistry())
}

func TestCounterInterface(t *testing.T) {
	setupTestRegistry()

	opts := CounterOpts{
		Name: "test_total",
		Help: "A test counter metric",
	}

	counter := NewCounter(opts, []string{"label1", "label2"})

	counter.Inc(Label{Name: "label1", Value: "value1"}, Label{Name: "label2", Value: "value2"})

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_test_total", &metricstest.ExpectedMetric{
		Labels: []Label{
			{Name: "label1", Value: "value1"},
			{Name: "label2", Value: "value2"},
		},
		Value: 1.0,
	})

	counter.Add(5.0, Label{Name: "label1", Value: "value1"}, Label{Name: "label2", Value: "value2"})
	gathered = metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_test_total", &metricstest.ExpectedMetric{
		Labels: []Label{
			{Name: "label1", Value: "value1"},
			{Name: "label2", Value: "value2"},
		},
		Value: 6.0,
	})

	counter.Reset()
	gathered = metricstest.MustGatherMetrics(t)
	// After reset, counter exists with value 0 and empty label values
	gathered.AssertMetric("kgateway_test_total", &metricstest.ExpectedMetric{
		Labels: []Label{
			{Name: "label1", Value: ""},
			{Name: "label2", Value: ""},
		},
		Value: 0.0,
	})
}

func TestCounterPartialLabels(t *testing.T) {
	setupTestRegistry()

	opts := CounterOpts{
		Name: "test_total",
		Help: "A test counter metric with partial labels",
	}

	counter := NewCounter(opts, []string{"label1", "label2", "label3"})

	// Test with only some labels provided.
	counter.Inc(Label{Name: "label3", Value: "value3"}, Label{Name: "label1", Value: "value1"})

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_test_total", &metricstest.ExpectedMetric{
		Labels: []Label{
			{Name: "label1", Value: "value1"},
			{Name: "label2", Value: ""},
			{Name: "label3", Value: "value3"},
		},
		Value: 1.0,
	})
}

func TestCounterNoLabels(t *testing.T) {
	setupTestRegistry()

	opts := CounterOpts{
		Name: "test_total",
		Help: "A test counter metric with no labels",
	}

	counter := NewCounter(opts, []string{})

	counter.Inc()
	counter.Add(2.5)

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_test_total", &metricstest.ExpectedMetric{
		Labels: []Label{},
		Value:  3.5,
	})
}

func TestCounterRegistrationPanic(t *testing.T) {
	setupTestRegistry()

	opts := CounterOpts{
		Name: "test_total",
		Help: "A test counter metric",
	}

	NewCounter(opts, []string{})

	// Attempting to create a counter with the same name should panic.
	assert.Panics(t, func() {
		NewCounter(opts, []string{})
	})
}

func TestHistogramInterface(t *testing.T) {
	setupTestRegistry()

	opts := HistogramOpts{
		Name: "test_duration_seconds",
		Help: "A test histogram metric",
	}

	histogram := NewHistogram(opts, []string{"label1", "label2"})

	histogram.Observe(1.5, Label{Name: "label1", Value: "value1"}, Label{Name: "label2", Value: "value2"})
	histogram.Observe(2.5, Label{Name: "label1", Value: "value1"}, Label{Name: "label2", Value: "value2"})

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetricHistogramValue("kgateway_test_duration_seconds", metricstest.HistogramMetricOutput{
		SampleCount: 2,
		SampleSum:   4.0,
	})
	gathered.AssertMetricLabels("kgateway_test_duration_seconds", []Label{
		{Name: "label1", Value: "value1"},
		{Name: "label2", Value: "value2"},
	})
	gathered.AssertHistogramBuckets("kgateway_test_duration_seconds", DefaultBuckets)

	histogram.Reset()
	gathered = metricstest.MustGatherMetrics(t)
	gathered.AssertMetricNotExists("kgateway_test_duration_seconds")
}

func TestHistogramBuckets(t *testing.T) {
	setupTestRegistry()

	testBuckets := []float64{0.1, 0.5, 1.0, 2.5, 5.0, 10.0}

	opts := HistogramOpts{
		Name:    "test_duration_seconds",
		Help:    "A test histogram metric",
		Buckets: testBuckets,
	}

	histogram := NewHistogram(opts, []string{"label1", "label2"})

	histogram.Observe(1.5, Label{Name: "label1", Value: "value1"}, Label{Name: "label2", Value: "value2"})

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetricHistogramValue("kgateway_test_duration_seconds", metricstest.HistogramMetricOutput{
		SampleCount: 1,
		SampleSum:   1.5,
	})
	gathered.AssertMetricLabels("kgateway_test_duration_seconds", []Label{
		{Name: "label1", Value: "value1"},
		{Name: "label2", Value: "value2"},
	})
	gathered.AssertHistogramBuckets("kgateway_test_duration_seconds", testBuckets)
}

func TestHistogramPartialLabels(t *testing.T) {
	setupTestRegistry()

	opts := HistogramOpts{
		Name: "test_duration_seconds_partial",
		Help: "A test histogram metric with partial labels",
	}

	histogram := NewHistogram(opts, []string{"label1", "label2", "label3"})

	// Test with only some labels provided.
	histogram.Observe(3.14, Label{Name: "label1", Value: "value1"}, Label{Name: "label3", Value: "value3"})

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetricHistogramValue("kgateway_test_duration_seconds_partial", metricstest.HistogramMetricOutput{
		SampleCount: 1,
		SampleSum:   3.14,
	})
	gathered.AssertMetricLabels("kgateway_test_duration_seconds_partial", []Label{
		{Name: "label1", Value: "value1"},
		{Name: "label2", Value: ""},
		{Name: "label3", Value: "value3"},
	})
}

func TestHistogramNoLabels(t *testing.T) {
	setupTestRegistry()

	opts := HistogramOpts{
		Name: "test_duration_seconds_no_labels",
		Help: "A test histogram metric with no labels",
	}

	histogram := NewHistogram(opts, []string{})

	histogram.Observe(0.5)
	histogram.Observe(1.5)
	histogram.Observe(7.0)

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetricHistogramValue("kgateway_test_duration_seconds_no_labels", metricstest.HistogramMetricOutput{
		SampleCount: 3,
		SampleSum:   9.0,
	})
}

func TestHistogramRegistrationPanic(t *testing.T) {
	setupTestRegistry()

	opts := HistogramOpts{
		Name: "test_duration_seconds_duplicate",
		Help: "A test histogram metric",
	}

	NewHistogram(opts, []string{})

	// Attempting to create a histogram with the same name should panic.
	assert.Panics(t, func() {
		NewHistogram(opts, []string{})
	})
}

func TestGaugeInterface(t *testing.T) {
	setupTestRegistry()

	opts := GaugeOpts{
		Name: "tests",
		Help: "A test gauge metric",
	}

	gauge := NewGauge(opts, []string{"label1", "label2"})

	labels := []Label{
		{Name: "label1", Value: "value1"},
		{Name: "label2", Value: "value2"},
	}

	gauge.Set(10.0, labels...)

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_tests", &metricstest.ExpectedMetric{
		Labels: labels,
		Value:  10.0,
	})

	gauge.Add(5.0, labels...)
	gathered = metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_tests", &metricstest.ExpectedMetric{
		Labels: labels,
		Value:  15.0,
	})

	gauge.Sub(3.0, labels...)
	gathered = metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_tests", &metricstest.ExpectedMetric{
		Labels: labels,
		Value:  12.0,
	})

	gauge.Reset()
	gathered = metricstest.MustGatherMetrics(t)
	gathered.AssertMetricNotExists("kgateway_tests")
}

func TestGaugePartialLabels(t *testing.T) {
	setupTestRegistry()

	opts := GaugeOpts{
		Name: "tests_partial",
		Help: "A test gauge metric with partial labels",
	}

	gauge := NewGauge(opts, []string{"label1", "label2", "label3"})

	// Test with only some labels provided.
	gauge.Set(42.0, Label{Name: "label3", Value: "value3"}, Label{Name: "label1", Value: "value1"})

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_tests_partial", &metricstest.ExpectedMetric{
		Labels: []Label{
			{Name: "label1", Value: "value1"},
			{Name: "label2", Value: ""},
			{Name: "label3", Value: "value3"},
		},
		Value: 42.0,
	})
}

func TestGaugeNoLabels(t *testing.T) {
	setupTestRegistry()

	opts := GaugeOpts{
		Name: "tests_no_labels",
		Help: "A test gauge metric with no labels",
	}

	gauge := NewGauge(opts, []string{})

	gauge.Set(100.0)
	gauge.Add(50.0)
	gauge.Sub(25.0)

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_tests_no_labels", &metricstest.ExpectedMetric{
		Labels: []Label{},
		Value:  125.0,
	})
}

func TestGaugeRegistrationPanic(t *testing.T) {
	setupTestRegistry()

	opts := GaugeOpts{
		Name: "tests_duplicate",
		Help: "A test gauge metric",
	}

	NewGauge(opts, []string{})

	// Attempting to create a gauge with the same name should panic.
	assert.Panics(t, func() {
		NewGauge(opts, []string{})
	})
}

func TestGetPromCollector(t *testing.T) {
	setupTestRegistry()

	counterOpts := CounterOpts{
		Name: "test_collector_total",
		Help: "A test counter for collector testing",
	}
	counter := NewCounter(counterOpts, []string{})
	counterCollector := GetPromCollector(counter)
	require.NotNil(t, counterCollector)
	assert.IsType(t, &prometheus.CounterVec{}, counterCollector)

	histogramOpts := HistogramOpts{
		Name: "test_collector_duration_seconds",
		Help: "A test histogram for collector testing",
	}
	histogram := NewHistogram(histogramOpts, []string{})
	histogramCollector := GetPromCollector(histogram)
	require.NotNil(t, histogramCollector)
	assert.IsType(t, &prometheus.HistogramVec{}, histogramCollector)

	gaugeOpts := GaugeOpts{
		Name: "test_collectors",
		Help: "A test gauge for collector testing",
	}
	gauge := NewGauge(gaugeOpts, []string{})
	gaugeCollector := GetPromCollector(gauge)
	require.NotNil(t, gaugeCollector)
	assert.IsType(t, &prometheus.GaugeVec{}, gaugeCollector)

	invalidCollector := GetPromCollector("invalid")
	assert.Nil(t, invalidCollector)
}

func TestValidateLabelsOrder(t *testing.T) {
	setupTestRegistry()

	opts := CounterOpts{
		Name: "test_label_order_total",
		Help: "A test counter for label order testing",
	}

	counter := NewCounter(opts, []string{"z_label", "a_label", "m_label"})

	// Provide labels in different order than defined.
	counter.Inc(
		Label{Name: "a_label", Value: "a_value"},
		Label{Name: "z_label", Value: "z_value"},
		Label{Name: "m_label", Value: "m_value"},
	)

	gathered := metricstest.MustGatherMetrics(t)
	// Labels are provided to the metric in the order they are defined, and gathered in alphabetical order.
	gathered.AssertMetric("kgateway_test_label_order_total", &metricstest.ExpectedMetric{
		Labels: []Label{
			{Name: "a_label", Value: "a_value"},
			{Name: "m_label", Value: "m_value"},
			{Name: "z_label", Value: "z_value"},
		},
		Value: 1.0,
	})
}

func TestLabelsWithEmptyValues(t *testing.T) {
	setupTestRegistry()

	opts := CounterOpts{
		Name: "test_empty_labels_total",
		Help: "A test counter for empty label testing",
	}

	counter := NewCounter(opts, []string{"label1", "label2", "label3"})

	counter.Inc(
		Label{Name: "label1", Value: ""},
		Label{Name: "label2", Value: "non_empty"},
		Label{Name: "label3", Value: ""},
	)

	gathered := metricstest.MustGatherMetrics(t)
	gathered.AssertMetric("kgateway_test_empty_labels_total", &metricstest.ExpectedMetric{
		Labels: []Label{
			{Name: "label1", Value: ""},
			{Name: "label2", Value: "non_empty"},
			{Name: "label3", Value: ""},
		},
		Value: 1.0,
	})
}

func TestActiveMetrics(t *testing.T) {
	setupTestRegistry()

	// Ensure metrics are active by default.
	assert.True(t, Active())

	SetActive(false)
	assert.False(t, Active())

	SetActive(true)
	assert.True(t, Active())
}

type exampleTypedLabels struct {
	Namespace   string
	GatewayName string
	Port        string
}

func (r exampleTypedLabels) toMetricsLabels() []Label {
	return []Label{
		{Name: "namespace", Value: r.Namespace},
		{Name: "gateway", Value: r.GatewayName},
		{Name: "port", Value: r.Port},
	}
}

func BenchmarkCounter(b *testing.B) {
	opts := CounterOpts{
		Name: "test_empty_labels_total",
		Help: "A test counter for empty label testing",
	}

	counter := NewCounter(opts, []string{"namespace", "gateway", "port"})
	tl := exampleTypedLabels{
		Namespace:   "apple",
		GatewayName: "banana",
		Port:        "avocado",
	}
	for b.Loop() {
		counter.Inc(tl.toMetricsLabels()...)
	}
}
