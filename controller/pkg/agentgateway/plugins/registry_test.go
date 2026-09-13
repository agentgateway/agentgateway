package plugins

import (
	"slices"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
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

func TestContestedResourceExtensionFields(t *testing.T) {
	resources := func() krt.Collection[ir.AgwResource] {
		return krt.NewStaticCollection[ir.AgwResource](nil, nil)
	}
	ancestors := func() krt.Collection[*utils.AncestorBackend] {
		return krt.NewStaticCollection[*utils.AncestorBackend](nil, nil)
	}

	cases := []struct {
		name string
		plug []AgwPlugin
		want []string
	}{
		{
			name: "no extensions",
			plug: []AgwPlugin{{}, {}},
		},
		{
			name: "disjoint fields are not contested",
			plug: []AgwPlugin{
				{AddResourceExtension: &AddResourcesPlugin{Binds: resources()}},
				{AddResourceExtension: &AddResourcesPlugin{Listeners: resources()}},
			},
		},
		{
			name: "same field from two plugins",
			plug: []AgwPlugin{
				{AddResourceExtension: &AddResourcesPlugin{Routes: resources()}},
				{AddResourceExtension: &AddResourcesPlugin{Routes: resources()}},
			},
			want: []string{"Routes"},
		},
		{
			name: "reported sorted",
			plug: []AgwPlugin{
				{AddResourceExtension: &AddResourcesPlugin{AncestorBackends: ancestors(), Binds: resources()}},
				{AddResourceExtension: &AddResourcesPlugin{AncestorBackends: ancestors(), Binds: resources()}},
			},
			want: []string{"AncestorBackends", "Binds"},
		},
		{
			// ParentResolvers accumulates by design, so two contributors is not a conflict.
			name: "parent resolvers are not contested",
			plug: []AgwPlugin{
				{AddResourceExtension: &AddResourcesPlugin{ParentResolvers: []ParentResolver{nil}}},
				{AddResourceExtension: &AddResourcesPlugin{ParentResolvers: []ParentResolver{nil}}},
			},
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := contestedResourceExtensionFields(tc.plug); !slices.Equal(got, tc.want) {
				t.Fatalf("expected %v, got %v", tc.want, got)
			}
		})
	}
}
