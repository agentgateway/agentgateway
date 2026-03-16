package agentgatewaysyncer

import (
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

type PolicyStatusCollections = map[schema.GroupKind]krt.StatusCollection[controllers.Object, any]

func AgwPolicyCollection(agwPlugins plugins.AgwPlugin, references plugins.ReferenceIndex, krtopts krtutil.KrtOptions) (krt.Collection[ir.AgwResource], krt.Collection[*plugins.PolicyAttachment], PolicyStatusCollections) {
	var allPolicies []krt.Collection[plugins.AgwPolicy]
	var allReferences []krt.Collection[*plugins.PolicyAttachment]
	policyStatusMap := PolicyStatusCollections{}
	// Collect all policies from registered plugins.
	// Note: Only one plugin should be used per source GVK.
	// Avoid joining collections per-GVK before passing them to a plugin.
	for gvk, plugin := range agwPlugins.ContributesPolicies {
		policy, policyStatus, refs := plugin.ApplyPolicies(plugins.PolicyPluginInput{References: references})
		if refs != nil {
			allReferences = append(allReferences, refs)
		}
		allPolicies = append(allPolicies, policy)
		if policyStatus != nil {
			// some plugins may not have a status collection (a2a services, etc.)
			policyStatusMap[gvk] = policyStatus
		}
	}
	joinPolicies := krt.JoinCollection(allPolicies, krtopts.ToOptions("JoinPolicies")...)

	allPoliciesCol := krt.NewCollection(joinPolicies, func(ctx krt.HandlerContext, i plugins.AgwPolicy) *ir.AgwResource {
		return ptr.Of(translator.ToResourceForGateway(*i.Gateway, i))
	}, krtopts.ToOptions("AllPolicies")...)

	allRefsCol := krt.JoinCollection(allReferences, krtopts.ToOptions("PolicyReferences")...)

	return allPoliciesCol, allRefsCol, policyStatusMap
}
