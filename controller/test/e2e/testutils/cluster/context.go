//go:build e2e

package cluster

import (
	kubelib "istio.io/istio/pkg/kube"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
)

// Context contains the metadata about a Kubernetes cluster
// It also includes useful utilities for interacting with that cluster
type Context struct {
	// The name of the Kubernetes cluster
	// The assumption is that when multiple clusters are running at once, they will each have unique names
	Name string

	// The context of the Kubernetes cluster
	KubeContext string

	// RestConfig holds the common attributes that can be passed to a Kubernetes client on initialization
	RestConfig *rest.Config

	// A CLI for interacting with Kubernetes cluster
	Cli *kubectl.Cli

	// A client to perform CRUD operations on the Kubernetes Cluster
	Client client.Client

	// A client to perform CRUD operations on the Kubernetes Cluster
	IstioClient kubelib.CLIClient

	// A set of clients for interacting with the Kubernetes Cluster
	Clientset *kubernetes.Clientset
}
