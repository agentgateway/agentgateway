package oidc

import (
	"fmt"
	"time"

	"k8s.io/apimachinery/pkg/api/equality"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

// OidcOwnerID identifies the Kubernetes resource that owns an OIDC discovery request.
type OidcOwnerID struct {
	Namespace string
	Name      string
	// Path is the spec path of the OIDC field within the owning resource.
	Path string
}

func (o OidcOwnerID) String() string {
	return fmt.Sprintf("AgentgatewayPolicy/%s/%s#%s", o.Namespace, o.Name, o.Path)
}

// RemoteOidcOwner identifies the Kubernetes owner (AgentgatewayPolicy) that
// triggered the OIDC discovery fetch and carries the configuration needed to
// resolve and perform the fetch.
type RemoteOidcOwner struct {
	ID               OidcOwnerID
	DefaultNamespace string
	Config           agentgateway.OIDC
	TTL              time.Duration
}

func (o RemoteOidcOwner) ResourceName() string {
	return o.ID.String()
}

func (o RemoteOidcOwner) Equals(other RemoteOidcOwner) bool {
	return o.ID == other.ID &&
		o.DefaultNamespace == other.DefaultNamespace &&
		o.TTL == other.TTL &&
		// equality.Semantic treats nil and zero-length slices as equal, so a
		// kubebuilder-defaulted empty Scopes list does not cause spurious
		// owner re-resolution against an unset Scopes field.
		equality.Semantic.DeepEqual(o.Config, other.Config)
}

// OwnersFromPolicy extracts RemoteOidcOwner values from an AgentgatewayPolicy
// that has a .spec.traffic.oidc field set.
func OwnersFromPolicy(policy *agentgateway.AgentgatewayPolicy) []RemoteOidcOwner {
	if len(policy.Spec.TargetRefs) == 0 {
		return nil
	}

	if policy.Spec.Traffic == nil {
		return nil
	}

	owner, ok := PolicyOIDCLookupOwner(policy.Namespace, policy.Name, policy.Spec.Traffic.OIDC)
	if !ok {
		return nil
	}

	return []RemoteOidcOwner{
		owner,
	}
}

func PolicyOIDCLookupOwner(namespace, name string, oidcCfg *agentgateway.OIDC) (RemoteOidcOwner, bool) {
	if oidcCfg == nil {
		return RemoteOidcOwner{}, false
	}

	return RemoteOidcOwner{
		ID: OidcOwnerID{
			Namespace: namespace,
			Name:      name,
			Path:      "spec.traffic.oidc",
		},
		DefaultNamespace: namespace,
		Config:           *oidcCfg.DeepCopy(),
		TTL:              TTLForOIDC(*oidcCfg),
	}, true
}

// TTLForOIDC returns the configured refresh interval for an OIDC provider,
// defaulting to 1 hour if not set.
func TTLForOIDC(cfg agentgateway.OIDC) time.Duration {
	if cfg.RefreshInterval == nil {
		return time.Hour
	}
	return cfg.RefreshInterval.Duration
}
