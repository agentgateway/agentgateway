package plugins

import (
	"maps"

	"k8s.io/apimachinery/pkg/runtime/schema"
)

type AgwPlugin struct {
	AddResourceExtension *AddResourcesPlugin
	ContributesPolicies  map[schema.GroupKind]PolicyPlugin
	ContributesBackends  map[schema.GroupKind]BackendPlugin
}

func MergePlugins(plug ...AgwPlugin) AgwPlugin {
	ret := AgwPlugin{
		ContributesPolicies: make(map[schema.GroupKind]PolicyPlugin),
		ContributesBackends: make(map[schema.GroupKind]BackendPlugin),
	}
	for _, p := range plug {
		// Merge contributed policies
		maps.Copy(ret.ContributesPolicies, p.ContributesPolicies)
		maps.Copy(ret.ContributesBackends, p.ContributesBackends)
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
			if p.AddResourceExtension.AncestorBackends != nil {
				ret.AddResourceExtension.AncestorBackends = p.AddResourceExtension.AncestorBackends
			}
			if p.AddResourceExtension.GatewayStatuses != nil {
				ret.AddResourceExtension.GatewayStatuses = p.AddResourceExtension.GatewayStatuses
			}
			ret.AddResourceExtension.ParentResolvers = append(ret.AddResourceExtension.ParentResolvers, p.AddResourceExtension.ParentResolvers...)
		}
	}
	return ret
}
