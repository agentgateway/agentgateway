package agentgatewaysyncer

import (
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/ir"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

type BackendStatusCollections = map[schema.GroupKind]krt.StatusCollection[controllers.Object, any]

func AgwBackendReferencesCollection(agwPlugins plugins.AgwPlugin, krtopts krtutil.KrtOptions) krt.Collection[*plugins.PolicyAttachment] {
	var allReferences []krt.Collection[*plugins.PolicyAttachment]

	for _, plugin := range agwPlugins.ContributesBackends {
		refs := plugin.BuildReferences()
		if refs != nil {
			allReferences = append(allReferences, refs)
		}
	}

	allRefsCol := krt.JoinCollection(allReferences, krtopts.ToOptions("BackendReferences")...)
	return allRefsCol
}

func AgwBackendCollection(agwPlugins plugins.AgwPlugin, references plugins.ReferenceIndex, krtopts krtutil.KrtOptions) (krt.Collection[ir.AgwResource], BackendStatusCollections) {
	var allBackends []krt.Collection[ir.AgwResource]
	policyStatusMap := PolicyStatusCollections{}
	// Collect all policies from registered plugins.
	// Note: Only one plugin should be used per source GVK.
	// Avoid joining collections per-GVK before passing them to a plugin.
	for gvk, plugin := range agwPlugins.ContributesBackends {
		policyStatus, policy := plugin.Build(plugins.PolicyPluginInput{References: references})
		allBackends = append(allBackends, policy)
		if policyStatus != nil {
			// some plugins may not have a status collection
			policyStatusMap[gvk] = policyStatus
		}
	}
	joinPolicies := krt.JoinCollection(allBackends, krtopts.ToOptions("JoinBackends")...)

	return joinPolicies, policyStatusMap
}
