//go:build e2e

package cluster

import (
	kubelib "istio.io/istio/pkg/kube"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

// Context contains the metadata about a Kubernetes cluster
// It also includes useful utilities for interacting with that cluster
type Context struct {
	// The name of the Kubernetes cluster
	// The assumption is that when multiple clusters are running at once, they will each have unique names
	Name string

	// The context of the Kubernetes cluster
	KubeContext string

	// A client to perform CRUD operations on the Kubernetes Cluster
	ControllerClient client.Client

	// A client to perform CRUD operations on the Kubernetes Cluster
	Client kubelib.CLIClient
}
