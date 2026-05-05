package deployer

import (
	"fmt"

	corev1 "k8s.io/api/core/v1"
)

// injectXdsCACertificate injects the CA certificate into Helm values so it can be used by proxy templates.
func injectXdsCACertificate(caCert string, vals *HelmConfig) error {
	if caCert == "" {
		return fmt.Errorf("xDS TLS is enabled but CA certificate is empty")
	}

	if vals.Agentgateway != nil {
		if vals.Agentgateway.Xds != nil && vals.Agentgateway.Xds.Tls != nil {
			vals.Agentgateway.Xds.Tls.CaCert = &caCert
		}
	}

	return nil
}

func extractXdsCACertificate(secret *corev1.Secret) (string, error) {
	caCert := secret.Data[corev1.ServiceAccountRootCAKey]
	if len(caCert) == 0 {
		caCert = secret.Data[corev1.TLSCertKey]
		if len(caCert) == 0 {
			return "", fmt.Errorf("xDS TLS secret %s/%s is missing ca.crt", secret.Namespace, secret.Name)
		}
	}
	return string(caCert), nil
}
