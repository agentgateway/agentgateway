package kubeutils

import (
	"fmt"

	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gwxv1a1 "sigs.k8s.io/gateway-api/apisx/v1alpha1"
)

func DetectListenerPortNumber(protocol gwv1.ProtocolType, port gwv1.PortNumber) (gwxv1a1.PortNumber, error) {
	if port != 0 {
		return port, nil
	}
	switch protocol {
	case gwv1.HTTPProtocolType:
		return 80, nil
	case gwv1.HTTPSProtocolType:
		return 443, nil
	}
	return 0, fmt.Errorf("protocol %v requires a port to be set", protocol)
}
