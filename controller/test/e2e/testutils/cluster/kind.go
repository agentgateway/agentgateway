//go:build e2e

package cluster

import (
	"os"

	kubelib "istio.io/istio/pkg/kube"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/client-go/kubernetes"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/log"
	"sigs.k8s.io/controller-runtime/pkg/log/zap"

	"github.com/kgateway-dev/kgateway/v2/pkg/schemes"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
	"github.com/kgateway-dev/kgateway/v2/test/testutils"
)

// MustKindContext returns the Context for a KinD cluster with the given name
func MustKindContext(clusterName string) *Context {
	return MustKindContextWithScheme(clusterName, schemes.GatewayScheme())
}

// MustKindContextWithScheme returns the Context for a KinD cluster with the given name and scheme
func MustKindContextWithScheme(clusterName string, scheme *runtime.Scheme) *Context {
	if len(clusterName) == 0 {
		// We fall back to the cluster named `kind` if no cluster name was provided
		clusterName = "kind"
	}

	kubeCtx := os.Getenv(testutils.KubeCtx)
	restCfg, err := kubeutils.GetRestConfigWithKubeContext(kubeCtx)
	if err != nil {
		panic(err)
	}

	clientset, err := kubernetes.NewForConfig(restCfg)
	if err != nil {
		panic(err)
	}

	// This line prevents controller-runtime from complaining about log.SetLogger never being called
	log.SetLogger(zap.New(zap.WriteTo(os.Stdout), zap.UseDevMode(true)))
	clt, err := client.New(restCfg, client.Options{
		Scheme: scheme,
	})
	if err != nil {
		panic(err)
	}

	istio, err := kubelib.NewCLIClient(kubelib.NewClientConfigForRestConfig(restCfg))
	if err != nil {
		panic(err)
	}
	istio.SetDefaultApplyNamespace("default")

	return &Context{
		Name:        clusterName,
		KubeContext: kubeCtx,
		RestConfig:  restCfg,
		Cli:         kubectl.NewCli().WithKubeContext(kubeCtx).WithReceiver(os.Stdout),
		Client:      clt,
		IstioClient: istio,
		Clientset:   clientset,
	}
}
