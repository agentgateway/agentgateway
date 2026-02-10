//go:build e2e

package inferenceextension

import (
	"net/http"
	"time"

	"github.com/onsi/gomega"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	_ "embed"

	testmatchers "github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
)

var (
	// testNS is the namespace used for e2e tests. Test data manifests are hardcoded for this namespace.
	testNS = "inf-ext-e2e"
	// vllmDeployName is the name of the vLLM deployment name
	vllmDeployName = "vllm-llama3-8b-instruct"
	// targetModelName is the model name value defined in an LLM request body.
	targetModelName = "food-review-1"
	// testRouteName is the test data HTTPRoute name used in tests
	testRouteName = "llm-route"
	// podRunTimeout is time required for a pod to reach a "Running" status
	podRunTimeout = 3 * time.Minute
	// gtwProgramTimeout is time required for the gateway to reach "Programmed" status
	gtwProgramTimeout = 60 * time.Second
	// vllmManifest is the manifest for the vLLM simulator model server
	//go:embed testdata/vllm.yaml
	vllmManifest []byte
	// poolManifest is the manifest for the InferencePool resource
	//go:embed testdata/pool.yaml
	poolManifest []byte
	// eppManifest is the manifest for the Endpoint Picker (EPP)
	//go:embed testdata/epp.yaml
	eppManifest []byte
	// gtwManifest is the manifest for the Gateway resource
	//go:embed testdata/gateway.yaml
	gtwManifest []byte
	// routeManifest is the manifest for the HTTPRoute resource
	//go:embed testdata/route.yaml
	routeManifest []byte
	// clientManifest is the manifest for the curl client
	//go:embed testdata/curl_pod.yaml
	clientManifest []byte

	// The Gateway resources created by kgateway
	gtwObjectMeta = metav1.ObjectMeta{
		Name:      "inference-gateway",
		Namespace: testNS,
	}
	gtwDeployment = &appsv1.Deployment{ObjectMeta: gtwObjectMeta}
	gtwService    = &corev1.Service{ObjectMeta: gtwObjectMeta}

	// The expected response when curl'ing the vLLM backend
	expectedVllmResp = &testmatchers.HttpResponse{
		StatusCode: http.StatusOK,
		// Use a Gomega matcher so that we assert that the response body CONTAINS the substring `"model":"<modelName>"`
		Body: gomega.ContainSubstring(`"model":"` + targetModelName + `"`),
	}
)
