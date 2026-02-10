//go:build e2e

package rate_limit

import (
	"path/filepath"

	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

const (
	// test namespace for proxy resources
	namespace = "kgateway-test"
	// test namespace for ratelimit resources
	extensionsNamespace = "kgateway-test-extensions"
	// test service name
	serviceName = "backend-0"
)

var (
	// paths to test manifests
	commonManifest            = getTestFile("common.yaml")
	simpleServiceManifest     = getTestFile("service.yaml")
	httpRoutesManifest        = getTestFile("routes.yaml")
	ipRateLimitManifest       = getTestFile("ip-rate-limit.yaml")
	pathRateLimitManifest     = getTestFile("path-rate-limit.yaml")
	userRateLimitManifest     = getTestFile("user-rate-limit.yaml")
	combinedRateLimitManifest = getTestFile("combined-rate-limit.yaml")
	rateLimitServerManifest   = getTestFile("rate-limit-server.yaml")

	// metadata for gateway - matches the name "gateway" from common.yaml
	gatewayObjectMeta = metav1.ObjectMeta{Name: "gateway", Namespace: namespace}
	gateway           = &gwv1.Gateway{
		ObjectMeta: gatewayObjectMeta,
	}

	// metadata for proxy resources
	proxyObjectMeta = metav1.ObjectMeta{Name: "gateway", Namespace: namespace}

	proxyDeployment = &appsv1.Deployment{
		ObjectMeta: proxyObjectMeta,
	}
	proxyService = &corev1.Service{
		ObjectMeta: proxyObjectMeta,
	}
	proxyServiceAccount = &corev1.ServiceAccount{
		ObjectMeta: proxyObjectMeta,
	}

	// metadata for backend service
	serviceMeta = metav1.ObjectMeta{
		Namespace: namespace,
		Name:      serviceName,
	}

	simpleSvc = &corev1.Service{
		ObjectMeta: serviceMeta,
	}

	simpleDeployment = &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      serviceName,
		},
	}

	// metadata for rate limit service
	rateLimitObjectMeta = metav1.ObjectMeta{Name: "ratelimit", Namespace: extensionsNamespace}

	rateLimitDeployment = &appsv1.Deployment{
		ObjectMeta: rateLimitObjectMeta,
	}
	rateLimitService = &corev1.Service{
		ObjectMeta: rateLimitObjectMeta,
	}
	rateLimitConfigMap = &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{Name: "ratelimit-config", Namespace: extensionsNamespace},
	}

	// metadata for httproutes
	route = &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      "test-route-1",
		},
	}

	route2 = &gwv1.HTTPRoute{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      "test-route-2",
		},
	}

	// AgentgatewayPolicy for different rate limit scenarios
	ipRateLimitAgentgatewayPolicy = &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      "ip-rate-limit",
		},
	}

	pathRateLimitAgentgatewayPolicy = &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      "path-rate-limit",
		},
	}

	userRateLimitAgentgatewayPolicy = &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      "user-rate-limit",
		},
	}

	combinedRateLimitAgentgatewayPolicy = &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: namespace,
			Name:      "combined-rate-limit",
		},
	}
)

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "global", "testdata", filename)
}
