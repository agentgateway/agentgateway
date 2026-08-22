package metrics_test

import (
	"runtime"
	"testing"

	"github.com/agentgateway/agentgateway/controller/pkg/metrics"
	"github.com/agentgateway/agentgateway/controller/pkg/metrics/metricstest"
	"github.com/agentgateway/agentgateway/controller/pkg/version"
)

func TestBuildInfoMetric(t *testing.T) {
	metrics.SetRegistry(false, metrics.NewRegistry())

	// Re-register the build info collector on the new test registry,
	// since SetRegistry replaced the global one.
	metrics.Registry().MustRegister(metrics.BuildInfoCollector())

	info := version.Info()
	gathered := metricstest.MustGatherMetrics(t)

	gathered.AssertMetric("agentgateway_controller_build_info", &metricstest.ExpectedMetric{
		Labels: []metrics.Label{
			{Name: "version", Value: info.Controller},
			{Name: "git_commit", Value: info.Commit},
			{Name: "build_date", Value: info.Date},
			{Name: "go_version", Value: runtime.Version()},
			{Name: "platform", Value: info.OS + "/" + info.Arch},
		},
		Value: 1.0,
	})
}
