package config

import (
	"context"
	"fmt"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
)

func extractConfigDump(kubeClient kubeutil.CLIClient, podName, podNamespace string, port int) ([]byte, error) {
	path := "config_dump"
	debug, err := kubeClient.AgentgatewayRequest(context.Background(), podName, podNamespace, "GET", path, port)
	if err != nil {
		return nil, fmt.Errorf("failed to execute command on %s.%s: %v", podName, podNamespace, err)
	}
	return debug, nil
}
