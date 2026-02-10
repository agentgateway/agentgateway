//go:build e2e

package metrics

import (
	"path/filepath"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	e2edefaults "github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
)

var (
	// manifests
	setupManifest                 = filepath.Join(fsutils.MustGetThisDir(), "testdata", "setup.yaml")
	setupCommonManifest           = filepath.Join(fsutils.MustGetThisDir(), "testdata", "setup-common.yaml")
	setupWithListenerSetsManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "setup-with-listenersets.yaml")

	// objects
	proxyObjectMeta = metav1.ObjectMeta{
		Name:      "gw1",
		Namespace: "default",
	}

	kgatewayMetricsObjectMeta = metav1.ObjectMeta{
		Name:      "kgateway-metrics",
		Namespace: "kgateway-test",
	}

	setup = base.TestCase{
		Manifests: []string{setupManifest, setupCommonManifest, e2edefaults.CurlPodManifest},
	}

	setupWithListenerSets = base.TestCase{
		Manifests: []string{setupWithListenerSetsManifest, setupCommonManifest, e2edefaults.CurlPodManifest},
	}

	// We only want to run one version of the metrics test because they will interfere with each other.
	testCases = map[string]*base.TestCase{
		"TestMetrics": {
			MaxGwApiVersion: base.GwApiRequireListenerSets,
		},
		"TestMetricsWithListenerSets": {
			MinGwApiVersion: base.GwApiRequireListenerSets,
		},
	}
)
