package syncer

import (
	"istio.io/istio/pkg/kube/krt"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/translator"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

type agentgatewaySyncerConfig struct {
	GatewayTransformationFunc   translator.GatewayTransformationFunction
	CustomResourceCollections   func(cfg CustomResourceCollectionsConfig)
	BuildAddressCollectionsFunc AgentgatewayAddressBuilderFunc
	BuildReferenceTypesFunc     func(agw *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes
	ExtraListenerSets           ExtraListenerSetsBuilderFunc
	AllowedListenersResolver    AllowedListenersResolver
}

type AgentgatewaySyncerOption func(*agentgatewaySyncerConfig)

func processAgentgatewaySyncerOptions(opts ...AgentgatewaySyncerOption) *agentgatewaySyncerConfig {
	cfg := &agentgatewaySyncerConfig{}
	for _, fn := range opts {
		fn(cfg)
	}
	return cfg
}

func WithGatewayTransformationFunc(f translator.GatewayTransformationFunction) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.GatewayTransformationFunc = f
		}
	}
}

func WithCustomResourceCollections(f func(cfg CustomResourceCollectionsConfig)) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.CustomResourceCollections = f
		}
	}
}

type AgentgatewayAddressBuilderFunc func(agw *plugins.AgwCollections, krtopts krtutil.KrtOptions) (krt.Collection[Address], func() bool)

// WithBuildAddressCollections provides a function to build the address collections for the syncer.
// This gives full control over how ServiceInfo and WorkloadInfo are constructed from the
// AgwCollections. The default implementation uses the istio ambient builder (see
// defaultBuildAddressCollections in syncer.go).
func WithBuildAddressCollections(f AgentgatewayAddressBuilderFunc) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.BuildAddressCollectionsFunc = f
		}
	}
}

func WithBuildReferenceTypes(f func(agw *plugins.AgwCollections, base plugins.ReferenceTypes) plugins.ReferenceTypes) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.BuildReferenceTypesFunc = f
		}
	}
}

type ExtraListenerSetsBuilderFunc func(agw *plugins.AgwCollections, krtopts krtutil.KrtOptions) krt.Collection[translator.ListenerSet]

// WithExtraListenerSets contributes listener sets from a source other than the Gateway API
// ListenerSet CRD. Once admitted they are indistinguishable from CRD-derived ones: conflict
// validation, GEP-1713 precedence, binds, route parents.
//
// Admission (see reviewExtraListenerSet) checks the parent Gateway's allowedListeners and that
// the contribution carries a ListenerSet listener's identity. Refusals go to
// Syncer.Outputs.RejectedListenerSets for the contributor to report; HasSynced does not cover
// that collection. Nothing else is checked: TLSInfo skips ReferenceGrant, and a zero
// ParentInfo.CreationTimestamp outranks every CRD ListenerSet in precedence.
func WithExtraListenerSets(f ExtraListenerSetsBuilderFunc) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.ExtraListenerSets = f
		}
	}
}

// AllowedListenersResolver returns a Gateway's listener attachment policy. nil denies all.
type AllowedListenersResolver func(gw *gwv1.Gateway) *gwv1.AllowedListeners

// WithAllowedListenersResolver supplies allowedListeners for Gateways whose CRD predates the
// field. Consulted only when spec.allowedListeners is unset, and only for contributed listener
// sets.
func WithAllowedListenersResolver(f AllowedListenersResolver) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.AllowedListenersResolver = f
		}
	}
}
