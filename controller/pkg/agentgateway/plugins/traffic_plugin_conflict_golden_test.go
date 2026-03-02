package plugins_test

import (
	"path/filepath"
	"regexp"
	"strings"
	"testing"

	"istio.io/istio/pilot/test/util"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test/util/file"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	"sigs.k8s.io/yaml"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
)

var goldenTimestampRegex = regexp.MustCompile(`lastTransitionTime:.*`)

func TestTrafficPolicyConflictGolden(t *testing.T) {
	runTrafficPolicyConflictGolden(t, "testdata/trafficpolicy-conflict/route-jwt-conflicts-gateway-oauth2.yaml", "route-jwt")
}

func runTrafficPolicyConflictGolden(t *testing.T, goldenFile, policyName string) {
	t.Helper()
	data := file.AsStringOrFail(t, goldenFile)
	idx := strings.Index(data, "---\n# Output")
	if idx == -1 {
		t.Fatalf("golden file %q missing output separator", goldenFile)
	}
	inputData := data[:idx-1]

	ctx := testutils.BuildMockPolicyContext(t, []any{inputData})
	var policy *agentgateway.AgentgatewayPolicy
	for _, candidate := range krt.Fetch(ctx.Krt, ctx.Collections.AgentgatewayPolicies) {
		if candidate.Namespace == "default" && candidate.Name == policyName {
			policy = candidate
			break
		}
	}
	if policy == nil {
		t.Fatalf("policy %q not found in fixture", policyName)
	}

	status, output := plugins.TranslateAgentgatewayPolicy(ctx.Krt, policy, ctx.Collections)
	marshaled, err := yaml.Marshal(struct {
		Status *gwv1.PolicyStatus  `json:"status,omitempty"`
		Output []plugins.AgwPolicy `json:"output"`
	}{
		Status: status,
		Output: output,
	})
	if err != nil {
		t.Fatalf("failed to marshal output: %v", err)
	}
	marshaled = goldenTimestampRegex.ReplaceAll(marshaled, []byte("lastTransitionTime: fake"))

	actual := inputData + "\n---\n# Output\n" + string(marshaled)
	if util.Refresh() {
		util.RefreshGoldenFile(t, []byte(actual), goldenFile)
	} else {
		util.CompareBytes(t, []byte(actual), []byte(data), filepath.Base(goldenFile))
	}
}
