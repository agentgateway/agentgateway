package config

import (
	"context"
	"fmt"

	"istio.io/istio/pkg/kube"
)

func extractConfigDump(kubeClient kube.CLIClient, podName, podNamespace string, port int) ([]byte, error) {
	path := "config_dump"
	debug, err := kubeClient.EnvoyDoWithPort(context.Background(), podName, podNamespace, "GET", path, port)
	if err != nil {
		return nil, fmt.Errorf("failed to execute command on %s.%s: %v", podName, podNamespace, err)
	}
	return debug, nil
}
