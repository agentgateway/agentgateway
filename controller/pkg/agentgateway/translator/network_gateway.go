package translator

import (
	"istio.io/istio/pkg/cluster"
	"istio.io/istio/pkg/network"
	"k8s.io/apimachinery/pkg/types"
)

// NetworkGateway represents a gateway that provides connectivity to a specific network
type NetworkGateway struct {
	// Network is the ID of the network this gateway provides access to
	Network network.ID
	// Cluster is the ID of the k8s cluster where this Gateway resides
	Cluster cluster.ID
	// Addr is the gateway address (IP or hostname)
	Addr string
	// Port is the gateway port
	Port uint32
	// HBONEPort indicates that the gateway supports HBONE on this port
	HBONEPort uint32
	// ServiceAccount the gateway runs as
	ServiceAccount types.NamespacedName
	// Source is the Gateway resource that this NetworkGateway was derived from
	Source types.NamespacedName
}

func (n NetworkGateway) ResourceName() string {
	return n.Source.String() + "/" + n.Addr
}

func (n NetworkGateway) Equals(other NetworkGateway) bool {
	return n.Network == other.Network &&
		n.Cluster == other.Cluster &&
		n.Addr == other.Addr &&
		n.Port == other.Port &&
		n.HBONEPort == other.HBONEPort &&
		n.ServiceAccount == other.ServiceAccount &&
		n.Source == other.Source
}
