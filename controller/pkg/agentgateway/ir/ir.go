package ir

import (
	"fmt"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pilot/pkg/util/protoconv"
	"k8s.io/apimachinery/pkg/types"

	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
)

var logger = logging.New("agentgateway")

func GetAgwResourceName(r *api.Resource) string {
	switch t := r.GetKind().(type) {
	case *api.Resource_Bind:
		return "bind/" + t.Bind.GetKey()
	case *api.Resource_Listener:
		return "listener/" + t.Listener.GetKey()
	case *api.Resource_Backend:
		return "backend/" + t.Backend.GetKey()
	case *api.Resource_Route:
		return "route/" + t.Route.GetKey()
	case *api.Resource_TcpRoute:
		return "tcp_route/" + t.TcpRoute.GetKey()
	case *api.Resource_Policy:
		return "policy/" + t.Policy.GetKey()
	default:
		logger.Error("unknown Agw resource", "type", fmt.Sprintf("%T", t))
		return "unknown/" + r.String()
	}
}

// AgwResource maps a specific resource to a Gateway instance.
// Gateway may be empty, which means it applies to all gateways
type AgwResource struct {
	Resource *api.Resource        `json:"resource"`
	Gateway  types.NamespacedName `json:"gateway,omitzero"`
}

func (g AgwResource) IntoProto() *api.Resource {
	return g.Resource
}

func (g AgwResource) ResourceName() string {
	if g.Gateway == (types.NamespacedName{}) {
		return GetAgwResourceName(g.Resource)
	}
	return g.Gateway.String() + "/" + GetAgwResourceName(g.Resource)
}

func (g AgwResource) XDSResourceName() string {
	return GetAgwResourceName(g.Resource)
}

func (g AgwResource) Equals(other AgwResource) bool {
	return protoconv.Equals(g.Resource, other.Resource) && g.Gateway == other.Gateway
}
