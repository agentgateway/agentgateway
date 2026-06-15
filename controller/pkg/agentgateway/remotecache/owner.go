package remotecache

import "fmt"

// OwnerKind identifies the resource kind that triggered a remote fetch;
// leads OwnerID.String() so persisted owner strings stay unambiguous.
type OwnerKind string

const (
	OwnerKindPolicy  OwnerKind = "AgentgatewayPolicy"
	OwnerKindBackend OwnerKind = "AgentgatewayBackend"
)

// OwnerID identifies the Kubernetes resource that triggers a remote fetch.
// Both the OIDC and JWKS subsystems use this type directly.
type OwnerID struct {
	Kind      OwnerKind
	Namespace string
	Name      string
	Path      string
}

func (o OwnerID) String() string {
	return fmt.Sprintf("%s/%s/%s#%s", o.Kind, o.Namespace, o.Name, o.Path)
}
