package plugins

import (
	"maps"

	"k8s.io/apimachinery/pkg/runtime/schema"
)

type AgwPlugin struct {
	AddResourceExtension *AddResourcesPlugin
	ContributesPolicies  map[schema.GroupKind]PolicyPlugin
	// extra has sync beyond primary resources in the collections above
	ExtraHasSynced func() bool
}

func MergePlugins(plug ...AgwPlugin) AgwPlugin {
	ret := AgwPlugin{
		ContributesPolicies: make(map[schema.GroupKind]PolicyPlugin),
	}
	var hasSynced []func() bool
	for _, p := range plug {
		// Merge contributed policies
		maps.Copy(ret.ContributesPolicies, p.ContributesPolicies)
		if p.AddResourceExtension != nil {
			if ret.AddResourceExtension == nil {
				ret.AddResourceExtension = &AddResourcesPlugin{}
			}
			if ret.AddResourceExtension.Binds == nil {
				ret.AddResourceExtension.Binds = p.AddResourceExtension.Binds
			}
			if p.AddResourceExtension.Listeners != nil {
				ret.AddResourceExtension.Listeners = p.AddResourceExtension.Listeners
			}
			if p.AddResourceExtension.Routes != nil {
				ret.AddResourceExtension.Routes = p.AddResourceExtension.Routes
			}
		}
		if p.ExtraHasSynced != nil {
			hasSynced = append(hasSynced, p.ExtraHasSynced)
		}
	}
	ret.ExtraHasSynced = mergeSynced(hasSynced)
	return ret
}

func mergeSynced(funcs []func() bool) func() bool {
	return func() bool {
		for _, f := range funcs {
			if !f() {
				return false
			}
		}
		return true
	}
}

// Plugins registers all built-in policy plugins
func Plugins(agw *AgwCollections) []AgwPlugin {
	return []AgwPlugin{
		NewAgentPlugin(agw),
		NewInferencePlugin(agw),
		NewA2APlugin(agw),
		NewBackendTLSPlugin(agw),
	}
}

func (p AgwPlugin) HasSynced() bool {
	for _, pol := range p.ContributesPolicies {
		if pol.Policies != nil && !pol.Policies.HasSynced() {
			return false
		}
	}
	if p.ExtraHasSynced != nil && !p.ExtraHasSynced() {
		return false
	}
	return true
}
