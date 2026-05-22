//go:build e2e

package base

import (
	"testing"

	"istio.io/istio/pkg/test"
	"k8s.io/apimachinery/pkg/runtime"

	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

var AgentgatewayControllerName = wellknown.DefaultAgwControllerName

var configureTest = func(t *testing.T) {}

var configureScheme = func(t test.Failer, scheme *runtime.Scheme) {}

var interceptManifestFiles = func(t test.Failer, tmpDir string, manifests ...string) []string {
	return manifests
}

func ConfigureTest(t *testing.T) {
	t.Helper()
	configureTest(t)
}
