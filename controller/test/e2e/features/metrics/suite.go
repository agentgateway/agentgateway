//go:build e2e

package metrics

import (
	"context"
	"net/http"
	"time"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/metrics"
	"github.com/kgateway-dev/kgateway/v2/pkg/metrics/metricstest"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	e2edefaults "github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

// testingSuite is a suite of basic control plane metrics.
type testingSuite struct {
	*base.BaseTestingSuite
}

// NewTestingSuite creates a new testing suite for control plane metrics.
func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases,
			base.WithSetupByVersion(map[base.GwApiChannel]map[base.GwApiVersion]*base.TestCase{
				base.GwApiChannelExperimental: {
					base.GwApiV1_3_0: &setupWithListenerSets, // ListenerSet available in experimental >= 1.3
				},
			}),
		),
	}
}

func (s *testingSuite) checkPodsRunning() {
	s.TestInstallation.AssertionsT(s.T()).EventuallyPodsRunning(s.Ctx, kgatewayMetricsObjectMeta.GetNamespace(), metav1.ListOptions{
		LabelSelector: e2edefaults.WellKnownAppLabel + "=kgateway",
	})
}

func (s *testingSuite) testMetrics(useListenerSets bool) {
	// Make sure pods are running.
	s.checkPodsRunning()

	// Verify the test services are created and working.
	s.TestInstallation.AssertionsT(s.T()).AssertEventualCurlResponse(
		s.Ctx,
		e2edefaults.CurlPodExecOpt,
		[]curl.Option{
			curl.WithHost(kubeutils.ServiceFQDN(proxyObjectMeta)),
			curl.WithHostHeader("example1.com"),
			curl.WithPort(8080),
		},
		&testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring(e2edefaults.NginxResponse),
		})

	// Verify the control plane metrics are generating as expected.
	s.TestInstallation.AssertionsT(s.T()).Assert.EventuallyWithT(func(c *assert.CollectT) {
		resp := s.TestInstallation.AssertionsT(s.T()).AssertEventualCurlReturnResponse(
			s.Ctx,
			e2edefaults.CurlPodExecOpt,
			[]curl.Option{
				curl.WithHost(kubeutils.ServiceFQDN(kgatewayMetricsObjectMeta)),
				curl.WithPort(9092),
				curl.WithPath("/metrics"),
			},
			&testmatchers.HttpResponse{StatusCode: http.StatusOK},
		)

		defer func() {
			if err := resp.Body.Close(); err != nil {
				c.Errorf("unable to close response body: %v", err)
			}
		}()

		gathered := metricstest.MustParseGatheredMetrics(c, resp.Body)

		// Assert xDS auth metrics
		var expectedGateways float64 = 2
		gathered.AssertMetric("kgateway_xds_auth_rq_total", &metricstest.ExpectedMetricValueTest{
			Test: metricstest.GreaterOrEqual(expectedGateways),
		})
		gathered.AssertMetric("kgateway_xds_auth_rq_success_total", &metricstest.ExpectedMetricValueTest{
			Test: metricstest.GreaterOrEqual(expectedGateways),
		})

		gathered.AssertMetricsLabelsInclude("kgateway_controller_reconcile_duration_seconds", [][]metrics.Label{{
			{Name: "controller", Value: "gateway"},
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
		}})

		gathered.AssertHistogramPopulated("kgateway_controller_reconcile_duration_seconds")

		gathered.AssertMetricsLabelsInclude("kgateway_controller_reconciliations_total", [][]metrics.Label{{
			{Name: "controller", Value: "gateway"},
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "result", Value: "success"},
		}})

		gathered.AssertMetricsInclude("kgateway_controller_reconciliations_running", []metricstest.ExpectMetric{
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "controller", Value: "gateway"},
					{Name: "name", Value: "gw1"},
					{Name: "namespace", Value: "default"},
				},
				Test: metricstest.Equal(0),
			},
		})

		gathered.AssertMetricsInclude("kgateway_resources_managed", []metricstest.ExpectMetric{
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "namespace", Value: "default"},
					{Name: "parent", Value: "gw1"},
					{Name: "resource", Value: "Gateway"},
				},
				Test: metricstest.Equal(1),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "namespace", Value: "default"},
					{Name: "parent", Value: "gw1"},
					{Name: "resource", Value: "HTTPRoute"},
				},
				Test: metricstest.Equal(2),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "namespace", Value: "default"},
					{Name: "parent", Value: "gw2"},
					{Name: "resource", Value: "Gateway"},
				},
				Test: metricstest.Equal(1),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "namespace", Value: "default"},
					{Name: "parent", Value: "gw2"},
					{Name: "resource", Value: "HTTPRoute"},
				},
				Test: metricstest.Equal(1),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "namespace", Value: "default"},
					{Name: "parent", Value: "ls1"},
					{Name: "resource", Value: "HTTPRoute"},
				},
				Test: metricstest.Equal(1),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "namespace", Value: "default"},
					{Name: "parent", Value: ""},
					{Name: "resource", Value: "HTTPListenerPolicy"},
				},
				Test: metricstest.Equal(1),
			},
		})

		if useListenerSets {
			gathered.AssertMetricsInclude("kgateway_resources_managed", []metricstest.ExpectMetric{
				&metricstest.ExpectedMetricValueTest{
					Labels: []metrics.Label{
						{Name: "namespace", Value: "default"},
						{Name: "parent", Value: "gw1"},
						{Name: "resource", Value: "XListenerSet"},
					},
					Test: metricstest.Equal(1),
				},
			})
		}

		gathered.AssertMetricsInclude("kgateway_resources_status_syncs_started_total", []metricstest.ExpectMetric{
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "Gateway"},
				},
				Test: metricstest.Between(1, 6),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "HTTPRoute"},
				},
				Test: metricstest.Between(1, 3),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: ""},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "HTTPListenerPolicy"},
				},
				Test: metricstest.Between(1, 2),
			},
		})

		gathered.AssertMetricsInclude("kgateway_resources_status_syncs_completed_total", []metricstest.ExpectMetric{
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "Gateway"},
				},
				Test: metricstest.Between(1, 6),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "HTTPRoute"},
				},
				Test: metricstest.Between(1, 3),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: ""},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "HTTPListenerPolicy"},
				},
				Test: metricstest.Between(1, 2),
			},
		})

		gathered.AssertMetricsLabelsInclude("kgateway_resources_status_sync_duration_seconds", [][]metrics.Label{{
			{Name: "gateway", Value: "gw2"},
			{Name: "namespace", Value: "default"},
			{Name: "resource", Value: "Gateway"},
		}, {
			{Name: "gateway", Value: "gw2"},
			{Name: "namespace", Value: "default"},
			{Name: "resource", Value: "HTTPRoute"},
		}, {
			{Name: "gateway", Value: ""},
			{Name: "namespace", Value: "default"},
			{Name: "resource", Value: "HTTPListenerPolicy"},
		}})

		gathered.AssertHistogramPopulated("kgateway_resources_status_sync_duration_seconds")

		gathered.AssertMetricsInclude("kgateway_xds_snapshot_syncs_total", []metricstest.ExpectMetric{
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw1"},
					{Name: "namespace", Value: "default"},
				},
				Test: metricstest.Between(1, 2),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
				},
				Test: metricstest.Between(1, 2),
			},
		})

		gathered.AssertMetricsLabelsInclude("kgateway_xds_snapshot_sync_duration_seconds", [][]metrics.Label{{
			{Name: "gateway", Value: "gw1"},
			{Name: "namespace", Value: "default"},
		}, {
			{Name: "gateway", Value: "gw2"},
			{Name: "namespace", Value: "default"},
		}})
		gathered.AssertHistogramPopulated("kgateway_xds_snapshot_sync_duration_seconds")

		gathered.AssertMetricsLabelsInclude("kgateway_xds_snapshot_transforms_total", [][]metrics.Label{{
			{Name: "gateway", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "result", Value: "success"},
		}})

		gathered.AssertMetricsLabelsInclude("kgateway_xds_snapshot_transform_duration_seconds", [][]metrics.Label{{
			{Name: "gateway", Value: "gw1"},
			{Name: "namespace", Value: "default"},
		}})

		gathered.AssertHistogramPopulated("kgateway_xds_snapshot_transform_duration_seconds")

		expectedGw1ListenerCount := 3
		expectedGw1RouteCount := 3
		if useListenerSets {
			expectedGw1ListenerCount = 4
			expectedGw1RouteCount = 4
		}
		gathered.AssertMetricsInclude("kgateway_xds_snapshot_resources", []metricstest.ExpectMetric{
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw1"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "Listener"},
				},
				Test: metricstest.Equal(float64(expectedGw1ListenerCount)),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw1"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "Route"},
				},
				Test: metricstest.Equal(float64(expectedGw1RouteCount)),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "Listener"},
				},
				Test: metricstest.Equal(2),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "resource", Value: "Route"},
				},
				Test: metricstest.Equal(2),
			},
		})

		gathered.AssertMetricsLabelsInclude("kgateway_status_syncer_status_syncs_total", [][]metrics.Label{{
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "result", Value: "success"},
			{Name: "syncer", Value: "GatewayStatusSyncer"},
		}})

		gathered.AssertMetricsLabelsInclude("kgateway_status_syncer_status_sync_duration_seconds", [][]metrics.Label{{
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "syncer", Value: "GatewayStatusSyncer"},
		}})

		gathered.AssertHistogramPopulated("kgateway_status_syncer_status_sync_duration_seconds")

		gathered.AssertMetricsLabelsInclude("kgateway_translator_translations_total", [][]metrics.Label{{
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "result", Value: "success"},
			{Name: "translator", Value: "TranslateGateway"},
		}, {
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "result", Value: "success"},
			{Name: "translator", Value: "TranslateHTTPRoute"},
		}})

		gathered.AssertMetricsLabelsInclude("kgateway_translator_translation_duration_seconds", [][]metrics.Label{{
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "translator", Value: "TranslateGateway"},
		}, {
			{Name: "name", Value: "gw1"},
			{Name: "namespace", Value: "default"},
			{Name: "translator", Value: "TranslateHTTPRoute"},
		}})

		gathered.AssertHistogramPopulated("kgateway_translator_translation_duration_seconds")

		gathered.AssertMetricsInclude("kgateway_routing_domains", []metricstest.ExpectMetric{
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw1"},
					{Name: "namespace", Value: "default"},
					{Name: "port", Value: "8080"},
				},
				Test: metricstest.Equal(5),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw1"},
					{Name: "namespace", Value: "default"},
					{Name: "port", Value: "8088"},
				},
				Test: metricstest.Equal(2),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw1"},
					{Name: "namespace", Value: "default"},
					{Name: "port", Value: "8443"},
				},
				Test: metricstest.Equal(2),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "port", Value: "8080"},
				},
				Test: metricstest.Equal(float64(3)),
			},
			&metricstest.ExpectedMetricValueTest{
				Labels: []metrics.Label{
					{Name: "gateway", Value: "gw2"},
					{Name: "namespace", Value: "default"},
					{Name: "port", Value: "8443"},
				},
				Test: metricstest.Equal(float64(3)),
			},
		})
	}, 20*time.Second, time.Second)
}

func (s *testingSuite) TestMetrics() {
	s.testMetrics(false)
}

func (s *testingSuite) TestMetricsWithListenerSets() {
	s.testMetrics(true)
}
