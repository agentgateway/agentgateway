package namespaces

import (
	"os"
)

const (
	DefaultNamespace = "agentgateway-system"
)

// GetPodNamespace returns the value of the env var `POD_NAMESPACE` and defaults to `agentgateway-system` if unset
func GetPodNamespace() string {
	if podNamespace := os.Getenv("POD_NAMESPACE"); podNamespace != "" {
		return podNamespace
	}
	return DefaultNamespace
}
