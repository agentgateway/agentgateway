package controller

import (
	"time"

	"k8s.io/apimachinery/pkg/types"

	"github.com/kgateway-dev/kgateway/v2/pkg/metrics"
)

const (
	controllerSubsystem = "controller"
	controllerNameLabel = "controller"
	namespaceLabel      = "namespace"
	nameLabel           = "name"
	resultLabel         = "result"
)

var (
	reconcileHistogramBuckets = []float64{0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5}
	reconciliationsTotal      = metrics.NewCounter(
		metrics.CounterOpts{
			Subsystem: controllerSubsystem,
			Name:      "reconciliations_total",
			Help:      "Total number of controller reconciliations",
		},
		[]string{controllerNameLabel, nameLabel, namespaceLabel, resultLabel},
	)
	reconcileDuration = metrics.NewHistogram(
		metrics.HistogramOpts{
			Subsystem:                       controllerSubsystem,
			Name:                            "reconcile_duration_seconds",
			Help:                            "Reconcile duration for controller",
			Buckets:                         reconcileHistogramBuckets,
			NativeHistogramBucketFactor:     1.1,
			NativeHistogramMaxBucketNumber:  100,
			NativeHistogramMinResetDuration: time.Hour,
		},
		[]string{controllerNameLabel, nameLabel, namespaceLabel},
	)
	reconciliationsRunning = metrics.NewGauge(
		metrics.GaugeOpts{
			Subsystem: controllerSubsystem,
			Name:      "reconciliations_running",
			Help:      "Number of reconciliations currently running",
		},
		[]string{controllerNameLabel, nameLabel, namespaceLabel},
	)
)

// collectReconciliationMetrics is called at the start of a controller reconciliation
// function to begin metrics collection and returns a function called at the end to
// complete metrics recording.
func collectReconciliationMetrics(controllerName string, req types.NamespacedName) func(error) {
	if !metrics.Active() {
		return func(err error) {}
	}

	start := time.Now()

	reconciliationsRunning.Add(1,
		metrics.Label{Name: controllerNameLabel, Value: controllerName},
		metrics.Label{Name: nameLabel, Value: req.Name},
		metrics.Label{Name: namespaceLabel, Value: req.Namespace},
	)

	return func(err error) {
		duration := time.Since(start)

		reconcileDuration.Observe(duration.Seconds(),
			metrics.Label{Name: controllerNameLabel, Value: controllerName},
			metrics.Label{Name: nameLabel, Value: req.Name},
			metrics.Label{Name: namespaceLabel, Value: req.Namespace},
		)

		result := "success"
		if err != nil {
			result = "error"
		}

		reconciliationsTotal.Inc(
			metrics.Label{Name: controllerNameLabel, Value: controllerName},
			metrics.Label{Name: nameLabel, Value: req.Name},
			metrics.Label{Name: namespaceLabel, Value: req.Namespace},
			metrics.Label{Name: resultLabel, Value: result},
		)

		reconciliationsRunning.Sub(1,
			metrics.Label{Name: controllerNameLabel, Value: controllerName},
			metrics.Label{Name: nameLabel, Value: req.Name},
			metrics.Label{Name: namespaceLabel, Value: req.Namespace},
		)
	}
}
