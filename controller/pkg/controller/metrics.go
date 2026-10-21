package controller

import (
	"github.com/agentgateway/agentgateway/controller/pkg/metrics"
)

const (
	controllerSubsystem = "controller"
	controllerNameLabel = "controller"
	resultLabel         = "result"
)

var reconciliationsTotal = metrics.NewCounter(
	metrics.CounterOpts{
		Subsystem: controllerSubsystem,
		Name:      "reconciliations_total",
		Help:      "Total number of controller reconciliations",
	},
	[]string{controllerNameLabel, resultLabel},
)

// recordReconciliation records the result of a controller reconciliation.
func recordReconciliation(controllerName string, err error) {
	if !metrics.Active() {
		return
	}
	result := "success"
	if err != nil {
		result = "error"
	}
	reconciliationsTotal.Inc(
		metrics.Label{Name: controllerNameLabel, Value: controllerName},
		metrics.Label{Name: resultLabel, Value: result},
	)
}
