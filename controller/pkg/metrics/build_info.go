package metrics

import (
	"runtime"

	"github.com/prometheus/client_golang/prometheus"

	"github.com/agentgateway/agentgateway/controller/pkg/version"
)

var buildInfoCollector = func() prometheus.Collector {
	info := version.Info()
	return prometheus.NewGaugeFunc(
		prometheus.GaugeOpts{
			Namespace: DefaultNamespace,
			Name:      "controller_build_info",
			Help:      "Agentgateway build metadata exposed as labels with a constant value of 1.",
			ConstLabels: prometheus.Labels{
				"version":    info.Controller,
				"git_commit": info.Commit,
				"build_date": info.Date,
				"go_version": runtime.Version(),
				"platform":   info.OS + "/" + info.Arch,
			},
		},
		func() float64 { return 1 },
	)
}()

// BuildInfoCollector returns the build info collector.
func BuildInfoCollector() prometheus.Collector {
	return buildInfoCollector
}

func init() {
	registry.MustRegister(buildInfoCollector)
}
