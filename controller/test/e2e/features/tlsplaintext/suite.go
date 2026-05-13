//go:build e2e

package tlsplaintext

import (
	"context"
	"net/http"
	"path/filepath"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e"
	testdefaults "github.com/agentgateway/agentgateway/controller/test/e2e/defaults"
	"github.com/agentgateway/agentgateway/controller/test/e2e/tests/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

var _ e2e.NewSuiteFunc = NewTestingSuite

type testingSuite struct {
	*base.BaseTestingSuite
}

var (
	basicGatewayManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "gw.yaml")

	gateway = &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw",
			Namespace: "default",
		},
	}
)

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	setup := base.TestCase{
		Manifests: []string{},
	}
	testCases := map[string]*base.TestCase{
		"TestTLSControlPlaneBasicFunctionality": {
			Manifests: []string{
				basicGatewayManifest,
			},
		},
	}
	return &testingSuite{
		base.NewBaseTestingSuite(ctx, testInst, setup, testCases),
	}
}

// TestTLSControlPlaneBasicFunctionality validates that the control plane with plaintext mode
// can successfully configure a basic Gateway and route traffic.
func (s *testingSuite) TestTLSControlPlaneBasicFunctionality() {
	s.TestInstallation.AssertionsT(s.T()).AssertEventualCurlResponse(
		s.Ctx,
		testdefaults.CurlPodExecOpt,
		[]curl.Option{
			curl.WithHost(kubeutils.ServiceFQDN(gateway.ObjectMeta)),
			curl.WithHostHeader("test.example.com"),
			curl.WithPort(8080),
			curl.WithPath("/headers"),
			curl.WithScheme("http"),
		},
		&matchers.HttpResponse{
			StatusCode: http.StatusOK,
			Body:       gomega.ContainSubstring("test.example.com"),
		},
	)
}
