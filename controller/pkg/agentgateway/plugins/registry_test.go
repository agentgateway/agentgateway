package plugins

import (
	"testing"

	"k8s.io/apimachinery/pkg/runtime/schema"
)

func TestMergePluginsMergesBackendContributions(t *testing.T) {
	backendGK := schema.GroupKind{Group: "enterpriseagentgateway.solo.io", Kind: "EnterpriseAgentgatewayBackend"}

	merged := MergePlugins(AgwPlugin{
		ContributesBackends: map[schema.GroupKind]BackendPlugin{
			backendGK: {},
		},
	})

	if _, ok := merged.ContributesBackends[backendGK]; !ok {
		t.Fatalf("expected backend contribution %v to be preserved", backendGK)
	}
}
