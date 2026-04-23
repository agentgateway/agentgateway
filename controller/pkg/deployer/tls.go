package deployer

import (
	"crypto/tls"
	"crypto/x509"
	"encoding/pem"
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
		tlsKey := secret.Data[corev1.TLSPrivateKeyKey]
		if len(tlsKey) == 0 {
			return "", fmt.Errorf("xDS TLS secret %s/%s with CA tls.crt must include tls.key", secret.Namespace, secret.Name)
		}
		if _, err := tls.X509KeyPair(caCert, tlsKey); err != nil {
			return "", fmt.Errorf("xDS TLS secret %s/%s has invalid tls.crt/tls.key: %w", secret.Namespace, secret.Name, err)
		}
		cert, err := parseXdsCertificate(caCert)
		if err != nil {
			return "", fmt.Errorf("xDS TLS secret %s/%s has invalid tls.crt: %w", secret.Namespace, secret.Name, err)
		}
		if !isXdsSigningCA(cert) {
			return "", fmt.Errorf("xDS TLS secret %s/%s with serving tls.crt/tls.key must include ca.crt", secret.Namespace, secret.Name)
		}
	}
	return string(caCert), nil
}

func parseXdsCertificate(certPEM []byte) (*x509.Certificate, error) {
	block, _ := pem.Decode(certPEM)
	if block == nil {
		return nil, fmt.Errorf("failed to parse certificate PEM")
	}
	return x509.ParseCertificate(block.Bytes)
}

func isXdsSigningCA(cert *x509.Certificate) bool {
	return cert.IsCA && cert.KeyUsage&x509.KeyUsageCertSign != 0
}
