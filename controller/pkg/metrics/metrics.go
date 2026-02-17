// Package metrics provides types for collecting and reporting metrics.
package metrics

import (
	"reflect"
	"sync"
	"sync/atomic"

	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/collectors"
	"sigs.k8s.io/controller-runtime/pkg/metrics"
)

const (
	// DefaultNamespace is the default namespace used for all metrics.
	DefaultNamespace = "kgateway"
)

var (
	// registry is the global metrics registry.
	registry     RegistererGatherer = prometheus.NewRegistry()
	registryLock                    = sync.RWMutex{}

	// DefaultBuckets defines the default buckets used for histograms.
	DefaultBuckets = prometheus.DefBuckets
)

func init() {
	registry.MustRegister(collectors.NewProcessCollector(collectors.ProcessCollectorOpts{}))
	registry.MustRegister(collectors.NewGoCollector())
}

// Metric defines a base interface for metrics.
type Metric interface {
	Labels() []string
}

// Label defines a name-value pair for labeling metrics.
type Label struct {
	Name  string
	Value string
}

// Counter defines the interface for a counter metric.
type Counter interface {
	Metric
	Inc(...Label)
	Add(float64, ...Label)
	Reset()
}

// prometheusCounter implements the Counter interface using the prometheus library.
type prometheusCounter struct {
	m      *prometheus.CounterVec
	labels []string
}

// CounterOpts defines options for creating a counter metric.
type CounterOpts prometheus.CounterOpts

// NewCounter creates a new counter metric.
func NewCounter(opts CounterOpts, labels []string) Counter {
	if opts.Namespace == "" {
		opts.Namespace = DefaultNamespace
	}

	c := &prometheusCounter{
		m:      prometheus.NewCounterVec(prometheus.CounterOpts(opts), labels),
		labels: labels,
	}

	registryLock.RLock()
	defer registryLock.RUnlock()

	if err := registry.Register(c.m); err != nil {
		panic("failed to register counter metric " + opts.Name + ": " + err.Error())
	}

	return c
}

// Labels returns names of the labels associated with the counter metric.
func (c *prometheusCounter) Labels() []string {
	return c.labels
}

// validateLabels validates a slice of Label values and converts to a slice of string values.
// It ensures that the order of labels is consistent with the order of the metric's labels.
// If a label is not present, it uses an empty string for that label.
func validateLabels(metric Metric, labels []Label) []string {
	values := make([]string, 0, len(labels))

	labelMap := make(map[string]string, len(labels))

	for _, label := range labels {
		labelMap[label.Name] = label.Value
	}

	for _, label := range metric.Labels() {
		if value, exists := labelMap[label]; exists {
			values = append(values, value)
		} else {
			values = append(values, "")
		}
	}

	return values
}

// validateLabels validates a slice of Label values for the counter metric.
func (c *prometheusCounter) validateLabels(labels []Label) []string {
	return validateLabels(c, labels)
}

// Inc increments the counter by 1.
func (c *prometheusCounter) Inc(labels ...Label) {
	c.m.WithLabelValues(c.validateLabels(labels)...).Inc()
}

// Add increments the counter by a given value.
func (c *prometheusCounter) Add(value float64, labels ...Label) {
	c.m.WithLabelValues(c.validateLabels(labels)...).Add(value)
}

// Reset resets the counter to zero and makes the metric visible in metrics
// gathering.
func (c *prometheusCounter) Reset() {
	c.m.Reset()
	// Initialize the counter to 0 so it's visible in metrics
	// gathering. Otherwise we're undoing some initialization like
	// StartResourceSyncMetricsProcessing() performs.
	c.Add(0)
}

// Histogram defines the interface for a histogram metric.
type Histogram interface {
	Metric
	Observe(float64, ...Label)
	Reset()
}

// prometheusHistogram implements the Histogram interface using the prometheus library.
type prometheusHistogram struct {
	m      *prometheus.HistogramVec
	labels []string
}

// HistogramOpts defines options for creating a histogram metric.
type HistogramOpts prometheus.HistogramOpts

// NewHistogram creates a new histogram metric.
func NewHistogram(opts HistogramOpts, labels []string) Histogram {
	if opts.Namespace == "" {
		opts.Namespace = DefaultNamespace
	}

	if len(opts.Buckets) == 0 {
		opts.Buckets = DefaultBuckets
	}

	h := &prometheusHistogram{
		m:      prometheus.NewHistogramVec(prometheus.HistogramOpts(opts), labels),
		labels: labels,
	}

	registryLock.RLock()
	defer registryLock.RUnlock()

	if err := registry.Register(h.m); err != nil {
		panic("failed to register histogram metric " + opts.Name + ": " + err.Error())
	}

	return h
}

// Labels returns the labels associated with the histogram metric.
func (c *prometheusHistogram) Labels() []string {
	return c.labels
}

// validateLabels validates a slice of Label values for the histogram metric.
func (h *prometheusHistogram) validateLabels(labels []Label) []string {
	return validateLabels(h, labels)
}

// Observe records a value in the histogram.
func (h *prometheusHistogram) Observe(value float64, labels ...Label) {
	h.m.WithLabelValues(h.validateLabels(labels)...).Observe(value)
}

// Reset resets the histogram to its initial state.
func (h *prometheusHistogram) Reset() {
	h.m.Reset()
}

// Gauge defines the interface for a gauge metric.
type Gauge interface {
	Metric
	Set(float64, ...Label)
	Add(float64, ...Label)
	Sub(float64, ...Label)
	Reset()
}

// prometheusGauge implements the Gauge interface using the prometheus library.
type prometheusGauge struct {
	m      *prometheus.GaugeVec
	labels []string
}

// GaugeOpts defines options for creating a gauge metric.
type GaugeOpts prometheus.GaugeOpts

// NewGauge creates a new gauge metric.
func NewGauge(opts GaugeOpts, labels []string) Gauge {
	if opts.Namespace == "" {
		opts.Namespace = DefaultNamespace
	}

	g := &prometheusGauge{
		m:      prometheus.NewGaugeVec(prometheus.GaugeOpts(opts), labels),
		labels: labels,
	}

	registryLock.RLock()
	defer registryLock.RUnlock()

	if err := registry.Register(g.m); err != nil {
		panic("failed to register gauge metric " + opts.Name + ": " + err.Error())
	}

	return g
}

// Labels returns the labels associated with the gauge metric.
func (c *prometheusGauge) Labels() []string {
	return c.labels
}

// validateLabels validates a slice of Label values for the gauge metric.
func (g *prometheusGauge) validateLabels(labels []Label) []string {
	return validateLabels(g, labels)
}

// Set sets the gauge to a specific value.
func (g *prometheusGauge) Set(value float64, labels ...Label) {
	g.m.WithLabelValues(g.validateLabels(labels)...).Set(value)
}

// Add increments the gauge by a given value.
func (g *prometheusGauge) Add(value float64, labels ...Label) {
	g.m.WithLabelValues(g.validateLabels(labels)...).Add(value)
}

// Sub decrements the gauge by a given value.
func (g *prometheusGauge) Sub(value float64, labels ...Label) {
	g.m.WithLabelValues(g.validateLabels(labels)...).Sub(value)
}

// Reset resets the gauge to zero.
func (g *prometheusGauge) Reset() {
	g.m.Reset()
}

// GetPromCollector returns the underlying collector for any valid prometheus metric.
// This is exported for testing purposes.
func GetPromCollector(c any) prometheus.Collector {
	switch c := c.(type) {
	case *prometheusCounter:
		return c.m
	case *prometheusHistogram:
		return c.m
	case *prometheusGauge:
		return c.m
	}

	return nil
}

// This allows metrics instrumentation to be globally enabled or disabled.
// By default, metrics are enabled.
// When disabled, metrics instrumentation will collapse into no-op statements,
// which can be useful for performance testing, or when metrics are not needed.
var (
	disabled uint32
)

// SetActive sets the globally active state for metrics.
// Setting this does not effect metrics that are already being collected.
func SetActive(active bool) {
	if active {
		atomic.StoreUint32(&disabled, 0)
	} else {
		atomic.StoreUint32(&disabled, 1)
	}
}

// Active checks if metrics are globally active.
func Active() bool {
	return atomic.LoadUint32(&disabled) == 0
}

// RegistererGatherer combines the Registerer and Gatherer interfaces from the
// Prometheus metrics library.
// These values can be used as metrics registries.
type RegistererGatherer metrics.RegistererGatherer

// Registry returns the global metrics registry.
func Registry() RegistererGatherer {
	registryLock.RLock()
	defer registryLock.RUnlock()

	return registry
}

// NewRegistry creates a new metrics registry.
func NewRegistry() RegistererGatherer {
	return prometheus.NewRegistry()
}

// SetRegistry sets the global metrics registry.
func SetRegistry(useBuiltinRegistry bool, r RegistererGatherer) {
	registryLock.Lock()
	defer registryLock.Unlock()

	if !useBuiltinRegistry {
		metrics.Registry = registry
	}

	if isNil(r) {
		registry = metrics.Registry
	} else {
		registry = r
	}
}

// isNil checks if the provided interface contains nil or a nil value.
func isNil(arg any) bool {
	if v := reflect.ValueOf(arg); !v.IsValid() || ((v.Kind() == reflect.Ptr ||
		v.Kind() == reflect.Interface ||
		v.Kind() == reflect.Slice ||
		v.Kind() == reflect.Map ||
		v.Kind() == reflect.Chan ||
		v.Kind() == reflect.Func) && v.IsNil()) {
		return true
	}

	return false
}
