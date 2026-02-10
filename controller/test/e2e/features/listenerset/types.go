//go:build e2e

package listenerset

import (
	"net/http"
	"path/filepath"

	"github.com/onsi/gomega/gstruct"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	e2edefaults "github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var (
	// manifests
	setupManifest                           = filepath.Join(fsutils.MustGetThisDir(), "testdata", "setup.yaml")
	validListenerSetManifest                = filepath.Join(fsutils.MustGetThisDir(), "testdata", "valid-listenerset.yaml")
	validListenerSetManifest2               = filepath.Join(fsutils.MustGetThisDir(), "testdata", "valid-listenerset-2.yaml")
	invalidListenerSetNotAllowedManifest    = filepath.Join(fsutils.MustGetThisDir(), "testdata", "invalid-listenerset-not-allowed.yaml")
	invalidListenerSetNonExistingGWManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "invalid-listenerset-non-existing-gw.yaml")
	conflictedListenerSetManifest           = filepath.Join(fsutils.MustGetThisDir(), "testdata", "conflicted-listenerset.yaml")
	policyManifest                          = filepath.Join(fsutils.MustGetThisDir(), "testdata", "policies.yaml")

	gwListener1Port  = 80
	gwListener2Port  = 8081
	ls1Listener1Port = 90
	ls1Listener2Port = 8091
	ls2Listener1Port = 8095
	ls3Listener1Port = 88

	proxyObjectMeta = metav1.ObjectMeta{
		Name:      "gw",
		Namespace: "default",
	}
	proxyService = &corev1.Service{ObjectMeta: proxyObjectMeta}

	// TestValidListenerSet
	validListenerSet = &gwxv1a1.XListenerSet{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "valid-ls",
			Namespace: "allowed-ns",
		},
	}

	// TestInvalidListenerSetNotAllowed
	invalidListenerSetNotAllowed = &gwxv1a1.XListenerSet{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "invalid-ls-not-allowed",
			Namespace: "curl",
		},
	}

	// TestInvalidListenerSetNonExistingGW
	invalidListenerSetNonExistingGW = &gwxv1a1.XListenerSet{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "invalid-ls-non-existing-gw",
			Namespace: "default",
		},
	}

	// TestConflictedListenerSet
	conflictedListenerSet = &gwxv1a1.XListenerSet{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "z-conflicted-listenerset",
			Namespace: "allowed-ns",
		},
	}

	expectOK = &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		Body:       gstruct.Ignore(),
	}

	expectOKWithCustomHeader = func(key, value string) *testmatchers.HttpResponse {
		return &testmatchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gstruct.Ignore(),
			Headers: map[string]any{
				key: value,
			},
		}
	}

	expectNotFound = &testmatchers.HttpResponse{
		StatusCode: http.StatusNotFound,
		Body:       gstruct.Ignore(),
	}

	curlExitErrorCode = 28

	setup = base.TestCase{
		Manifests: []string{e2edefaults.CurlPodManifest, setupManifest},
	}

	// test cases
	testCases = map[string]*base.TestCase{
		"TestValidListenerSet": {
			Manifests: []string{validListenerSetManifest},
		},
		"TestInvalidListenerSetNotAllowed": {
			Manifests: []string{invalidListenerSetNotAllowedManifest},
		},
		"TestInvalidListenerSetNonExistingGW": {
			Manifests: []string{invalidListenerSetNonExistingGWManifest},
		},
		"TestPolicies": {
			Manifests: []string{validListenerSetManifest, validListenerSetManifest2, policyManifest},
		},
		"TestConflictedListenerSet": {
			Manifests: []string{validListenerSetManifest, conflictedListenerSetManifest},
		},
	}
)
