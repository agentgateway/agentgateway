package remotecache

import "fmt"

// OwnerKind identifies which Kubernetes resource kind triggered a remote
// fetch. It is the leading segment of OwnerID.String() so persisted owner
// strings stay unambiguous across kinds.
type OwnerKind string

const (
	OwnerKindPolicy  OwnerKind = "AgentgatewayPolicy"
	OwnerKindBackend OwnerKind = "AgentgatewayBackend"
)

// OwnerID identifies the Kubernetes resource that triggers a remote fetch.
// Subsystems alias this type (oidc.OidcOwnerID, jwks.JwksOwnerID) for
// readable local naming without duplicating the layout.
type OwnerID struct {
	Kind      OwnerKind
	Namespace string
	Name      string
	Path      string
}

func (o OwnerID) String() string {
	return fmt.Sprintf("%s/%s/%s#%s", o.Kind, o.Namespace, o.Name, o.Path)
}
