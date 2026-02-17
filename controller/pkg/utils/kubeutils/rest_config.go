package kubeutils

import (
	"os"
	"strconv"

	"k8s.io/client-go/rest"
	"sigs.k8s.io/controller-runtime/pkg/client/config"
)

const (
	K8sClientQpsEnv     = "K8S_CLIENT_QPS"
	K8sClientQpsDefault = 50 // 10x the k8s-recommended default; kgateway gets busy writing status updates

	K8sClientBurstEnv     = "K8S_CLIENT_BURST"
	K8sClientBurstDefault = 100 // 10x the k8s-recommended default; kgateway gets busy writing status updates
)

// GetRestConfigWithKubeContext provides the rest.Config object for a given Kubernetes context
// This is a wrapper on the controller-runtime implementation, and allows overriding certain
// client properties via environment variables
func GetRestConfigWithKubeContext(kubeContext string) (*rest.Config, error) {
	restConfig, err := config.GetConfigWithContext(kubeContext)
	if err != nil {
		return nil, err
	}

	if err = setClientQpsOrError(restConfig); err != nil {
		return nil, err
	}
	if err = setClientBurstOrError(restConfig); err != nil {
		return nil, err
	}

	return restConfig, nil
}

func setClientQpsOrError(restConfig *rest.Config) error {
	restConfig.QPS = K8sClientQpsDefault
	clientQpsOverride := os.Getenv(K8sClientQpsEnv)
	if clientQpsOverride == "" {
		return nil
	}

	qps, err := strconv.ParseFloat(clientQpsOverride, 32)
	if err != nil {
		return err
	}

	restConfig.QPS = float32(qps)
	return nil
}

func setClientBurstOrError(restConfig *rest.Config) error {
	restConfig.Burst = K8sClientBurstDefault
	clientBurstOverride := os.Getenv(K8sClientBurstEnv)
	if clientBurstOverride == "" {
		return nil
	}

	burst, err := strconv.Atoi(clientBurstOverride)
	if err != nil {
		return err
	}
	restConfig.Burst = burst
	return nil
}
