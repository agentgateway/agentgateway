//go:build e2e

package inferenceextension

import (
	"context"
	"fmt"
	"testing"

	"github.com/stretchr/testify/suite"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

// testingSuite is the entire Suite of tests for testing K8s Service-specific features/fixes
type testingSuite struct {
	suite.Suite

	ctx context.Context

	// testInstallation contains all the metadata/utilities necessary to execute a series of tests
	// against a kgateway installation
	testInstallation *e2e.TestInstallation

	// manifests is a map of manifests keyed by a test name
	manifests map[string][][]byte
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	return &testingSuite{
		ctx:              ctx,
		testInstallation: testInst,
		manifests:        map[string][][]byte{},
	}
}

func (s *testingSuite) TestHTTPRouteWithInferencePool() {
	testName := "TestHTTPRouteWithInferencePool"

	testutils.Cleanup(s.T(), func() {
		manifests, ok := s.manifests[testName]
		if !ok {
			s.FailNow("no manifests found for %s", testName)
		}

		for _, m := range manifests {
			err := s.testInstallation.Actions.Kubectl().Delete(s.ctx, m)
			s.NoError(err, "can delete manifest %s", m)
		}
	})

	// Add the testdata manifests to the manifests map
	s.manifests = map[string][][]byte{
		testName: {
			clientManifest,
			vllmManifest,
			gtwManifest,
			poolManifest,
			eppManifest,
			routeManifest,
		},
	}

	// Apply the testdata manifests
	for _, m := range s.manifests[testName] {
		err := s.testInstallation.Actions.Kubectl().Apply(s.ctx, m)
		s.NoError(err, "can apply manifest %s", m)
	}

	// Assert test pods are running using key=pod_name and value=pod_namespace map.
	for k, v := range map[string]string{
		vllmDeployName:          testNS,
		vllmDeployName + "-epp": testNS,
		"curl":                  "curl"} {
		s.testInstallation.AssertionsT(s.T()).EventuallyPodsRunning(s.ctx, v, metav1.ListOptions{
			LabelSelector: "app=" + k,
		}, podRunTimeout)
	}

	// Assert gateway service and deployment are created
	s.testInstallation.AssertionsT(s.T()).EventuallyObjectsExist(s.ctx, gtwService, gtwDeployment)

	// Assert gateway programmed condition
	s.testInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.ctx,
		gtwObjectMeta.Name,
		gtwObjectMeta.Namespace,
		gwv1.GatewayConditionProgrammed,
		metav1.ConditionTrue,
		gtwProgramTimeout,
	)

	conditions := []gwv1.RouteConditionType{gwv1.RouteConditionAccepted, gwv1.RouteConditionResolvedRefs}
	for _, c := range conditions {
		s.testInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
			s.ctx,
			testRouteName,
			testNS,
			c,
			metav1.ConditionTrue,
		)
	}

	s.testInstallation.AssertionsT(s.T()).EventuallyInferencePoolCondition(
		s.ctx,
		vllmDeployName,
		testNS,
		inf.InferencePoolConditionAccepted,
		metav1.ConditionTrue,
	)

	s.testInstallation.AssertionsT(s.T()).EventuallyInferencePoolCondition(
		s.ctx,
		vllmDeployName,
		testNS,
		inf.InferencePoolConditionResolvedRefs,
		metav1.ConditionTrue,
	)

	// Exercise OpenAI API endpoint test cases
	type apiTest struct {
		api              string
		promptOrMessages string
	}

	tests := []apiTest{
		// Call with a single "prompt" field
		{
			api:              "/v1/completions",
			promptOrMessages: "Write as if you were a critic: San Francisco",
		},
		// Call with one user message
		{
			api:              "/v1/chat/completions",
			promptOrMessages: `[{"role":"user","content":"Write as if you were a critic: San Francisco"}]`,
		},
		// Call with a user–assistant–user message sequence
		{
			api: "/v1/chat/completions",
			promptOrMessages: `[{"role":"user","content":"Write as if you were a critic: San Francisco"},` +
				`{"role":"assistant","content":"Okay, let's see..."},` +
				`{"role":"user","content":"Now summarize your thoughts."}]`,
		},
	}

	for i := range tests {
		tc := tests[i]
		testName := fmt.Sprintf("CurlTestCase%d", i)

		s.T().Run(testName, func(t *testing.T) {
			// Build the "prompt" or "messages" fragment of the request body.
			var fieldJSON string
			if tc.api == "/v1/completions" {
				fieldJSON = fmt.Sprintf(`"prompt":"%s"`, tc.promptOrMessages)
			} else {
				fieldJSON = fmt.Sprintf(`"messages":%s`, tc.promptOrMessages)
			}

			// Inject that field into the rest of the body template
			body := fmt.Sprintf(
				`{"model":"%s",%s,"max_tokens":100,"temperature":0}`,
				targetModelName,
				fieldJSON,
			)

			// Assert expected curl response
			s.testInstallation.AssertionsT(s.T()).AssertEventualCurlResponse(
				s.ctx,
				defaults.CurlPodExecOpt,
				[]curl.Option{
					curl.WithHost(kubeutils.ServiceFQDN(gtwService.ObjectMeta)),
					curl.WithHeader("Content-Type", "application/json"),
					curl.WithPath(tc.api),
					curl.WithBody(body),
				},
				expectedVllmResp,
			)
		})
	}
}
