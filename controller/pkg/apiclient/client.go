package apiclient

import (
	"istio.io/istio/pkg/cluster"
	"istio.io/istio/pkg/kube"
	"k8s.io/client-go/rest"

	"github.com/agentgateway/agentgateway/controller/pkg/client/clientset/versioned"
)

var _ Client = (*client)(nil)

type Client interface {
	kube.Client
	Core() kube.Client
	Kgateway() versioned.Interface
}

type client struct {
	kube.Client
	kgateway versioned.Interface
}

func New(restConfig *rest.Config, clusterID string) (*client, error) {
	restCfg := kube.NewClientConfigForRestConfig(restConfig)
	kubeClient, err := kube.NewClient(restCfg, cluster.ID(clusterID))
	if err != nil {
		return nil, err
	}
	cli, err := versioned.NewForConfig(restConfig)
	if err != nil {
		return nil, err
	}

	RegisterTypes()
	kube.EnableCrdWatcher(kubeClient)
	return &client{
		Client:   kubeClient,
		kgateway: cli,
	}, nil
}

func (c *client) Kgateway() versioned.Interface {
	return c.kgateway
}

func (c *client) Core() kube.Client {
	return c.Client
}
