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

// ExtraListenerSetsBuilderFunc builds listener sets to contribute to the syncer. It matches the
// shape of AgentgatewayAddressBuilderFunc: krtopts is the syncer's, which is not necessarily
// agw.KrtOpts.
type ExtraListenerSetsBuilderFunc func(agw *plugins.AgwCollections, krtopts krtutil.KrtOptions) krt.Collection[translator.ListenerSet]

// WithExtraListenerSets provides a function returning listener sets to contribute to the syncer.
// The returned collection is joined with the one built from Gateway API ListenerSets, so
// contributed listeners take part in listener conflict validation, GEP-1713 precedence, bind
// construction and route parent resolution on the same terms as Gateway API ones.
//
// The syncer enforces two things on a contribution, and only two. The parent Gateway's
// allowedListeners must admit ParentInfo.SectionName's namespace. And the contribution must be
// addressable on its own identity: ParentInfo.SectionName must be set, Name must equal
// utils.InternalGatewayName(Parent.Namespace, Parent.Name, ParentInfo.SectionName),
// ParentInfo.ListenerKey must equal Name, ParentInfo.ParentGateway must equal GatewayParent, and
// Parent must name neither a Gateway nor a ListenerSet. Those keys are shared with Gateway API
// objects and drive status, policy attachment, route attachment and bind grouping, so a
// contribution that keys itself as something it does not own is attributed to that owner.
//
// Everything else a Gateway API ListenerSet is checked for is the contributor's responsibility,
// because the contributed form arrives already translated: TLSInfo is raw key material and is
// subject to no ReferenceGrant check, Internal is taken at face value, and protocol, hostname
// and gateway class ownership are not revalidated. A zero ParentInfo.CreationTimestamp wins
// GEP-1713 precedence over every Gateway API ListenerSet, including for a contested port's bind
// mode.
//
// Contributions failing the above are dropped and surfaced on
// Syncer.Outputs.RejectedListenerSets for the contributor to report against the resource it
// derived them from; the syncer cannot report it, as the contributed form carries no kind. That
// collection is not covered by Syncer.HasSynced.
func WithExtraListenerSets(f ExtraListenerSetsBuilderFunc) AgentgatewaySyncerOption {
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
