package agentgatewaysyncer

import (
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/ir"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/plugins"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/translator"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
)

type PolicyStatusCollections = map[schema.GroupKind]krt.StatusCollection[controllers.Object, gwv1.PolicyStatus]

func AgwPolicyCollection(agwPlugins plugins.AgwPlugin, krtopts krtutil.KrtOptions) (krt.Collection[ir.AgwResource], PolicyStatusCollections) {
	var allPolicies []krt.Collection[plugins.AgwPolicy]
	policyStatusMap := PolicyStatusCollections{}
	// Collect all policies from registered plugins.
	// Note: Only one plugin should be used per source GVK.
	// Avoid joining collections per-GVK before passing them to a plugin.
	for gvk, plugin := range agwPlugins.ContributesPolicies {
		policy, policyStatus := plugin.ApplyPolicies()
		allPolicies = append(allPolicies, policy)
		if policyStatus != nil {
			// some plugins may not have a status collection (a2a services, etc.)
			policyStatusMap[gvk] = policyStatus
		}
	}
	joinPolicies := krt.JoinCollection(allPolicies, krtopts.ToOptions("JoinPolicies")...)

	allPoliciesCol := krt.NewCollection(joinPolicies, func(ctx krt.HandlerContext, i plugins.AgwPolicy) *ir.AgwResource {
		return ptr.Of(translator.ToResourceGlobal(i))
	}, krtopts.ToOptions("AllPolicies")...)

	return allPoliciesCol, policyStatusMap
}
