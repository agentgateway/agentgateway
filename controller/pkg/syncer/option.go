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
	ExtraListenerSets           func(agw *plugins.AgwCollections) krt.Collection[translator.ListenerSet]
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

// WithExtraListenerSets provides a function returning listener sets to contribute to the
// syncer. The returned collection is joined with the one built from Gateway API ListenerSets,
// so contributed listeners take part in listener conflict validation, bind construction and
// route parent resolution on the same terms as Gateway API ones, and are gated by the parent
// Gateway's allowedListeners on the same terms.
//
// A contributed set must set Name to utils.InternalGatewayName(Parent.Namespace, Parent.Name,
// ParentInfo.SectionName), and Parent must not name a Gateway API ListenerSet. Listener set
// status and ListenerSet-targeted policy are keyed on Parent and SectionName rather than Name,
// so a set that aliases another owner's key there would be attributed to it.
//
// Contributions failing any of this are dropped and surfaced on
// Syncer.Outputs.RejectedListenerSets for the contributor to report against the resource it
// derived them from; the syncer cannot report it, as the contributed form carries no kind.
func WithExtraListenerSets(f func(agw *plugins.AgwCollections) krt.Collection[translator.ListenerSet]) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.ExtraListenerSets = f
		}
	}
}

// AllowedListenersResolver returns the listener attachment policy for a Gateway. Gateway.spec.
// allowedListeners only exists on Gateway API CRDs that ship the ListenerSet API, so on a
// cluster below that version there is no way to express the policy in the spec, and a resolver
// is the seam for sourcing it from somewhere that is available there.
//
// Returning nil denies all attachment, the same as an unset spec.allowedListeners.
type AllowedListenersResolver func(gw *gwv1.Gateway) *gwv1.AllowedListeners

// WithAllowedListenersResolver replaces the source of the policy applied to contributed listener
// sets, not its evaluation: the resolved value is interpreted by
// translator.AllowedListenersAcceptNamespace exactly as the spec field is. Gateway API
// ListenerSets are always governed by Gateway.spec.allowedListeners.
func WithAllowedListenersResolver(f AllowedListenersResolver) AgentgatewaySyncerOption {
	return func(o *agentgatewaySyncerConfig) {
		if f != nil {
			o.AllowedListenersResolver = f
		}
	}
}
