package oidc

import (
	"time"

	"k8s.io/apimachinery/pkg/api/equality"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
)

// OidcRefreshInterval is fixed (not user-tunable); conservative vs day-scale
// IdP key rotation overlap windows.
const OidcRefreshInterval = time.Hour

// RemoteOidcOwner is the AgentgatewayPolicy that triggered the OIDC discovery
// fetch, plus the config needed to perform it.
type RemoteOidcOwner struct {
	ID     remotecache.OwnerID
	Config agentgateway.OIDC
	TTL    time.Duration
}

func (o RemoteOidcOwner) ResourceName() string {
	return o.ID.String()
}

func (o RemoteOidcOwner) Equals(other RemoteOidcOwner) bool {
	return o.ID == other.ID &&
		o.TTL == other.TTL &&
		equality.Semantic.DeepEqual(o.Config, other.Config)
}

// OwnerFromPolicy returns the RemoteOidcOwner from a policy with
// .spec.traffic.oidc set; the CRD permits at most one OIDC per policy.
func OwnerFromPolicy(policy *agentgateway.AgentgatewayPolicy) (RemoteOidcOwner, bool) {
	if len(policy.Spec.TargetRefs) == 0 && len(policy.Spec.TargetSelectors) == 0 {
		return RemoteOidcOwner{}, false
	}

	if policy.Spec.Traffic == nil {
		return RemoteOidcOwner{}, false
	}

	return PolicyOIDCLookupOwner(policy.Namespace, policy.Name, policy.Spec.Traffic.OIDC)
}

func PolicyOIDCLookupOwner(namespace, name string, oidcCfg *agentgateway.OIDC) (RemoteOidcOwner, bool) {
	if oidcCfg == nil {
		return RemoteOidcOwner{}, false
	}

	return RemoteOidcOwner{
		ID: remotecache.OwnerID{
			Kind:      remotecache.OwnerKindPolicy,
			Namespace: namespace,
			Name:      name,
			Path:      "spec.traffic.oidc",
		},
		Config: *oidcCfg,
		TTL:    OidcRefreshInterval,
	}, true
}
