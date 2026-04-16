package utils

import (
	"fmt"

	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"
)

// InferencePoolBackendPort returns the canonical service port used when
// representing an InferencePool as a single logical backend in agentgateway.
func InferencePoolBackendPort(pool *inf.InferencePool) (uint32, error) {
	if len(pool.Spec.TargetPorts) == 0 {
		return 0, fmt.Errorf("inferencePool.targetPorts must contain at least one entry")
	}
	return uint32(pool.Spec.TargetPorts[0].Number), nil //nolint:gosec // G115: validated 1-65535
}
