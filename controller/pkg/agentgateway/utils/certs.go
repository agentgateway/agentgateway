package utils

import (
	"crypto/tls"
	"errors"

	corev1 "k8s.io/api/core/v1"
	"k8s.io/client-go/util/cert"
)

var ErrMissingCACertKey = errors.New("ca.crt key missing")

// ValidateTLSKeyPair verifies that the supplied certificate and key are a valid pair.
func ValidateTLSKeyPair(certChain, privateKey []byte) error {
	_, err := tls.X509KeyPair(certChain, privateKey)
	return err
}

// NormalizePEMCerts parses and re-encodes a PEM certificate bundle into a stable format.
func NormalizePEMCerts(pemBytes []byte) ([]byte, error) {
	candidateCert, err := cert.ParseCertsPEM(pemBytes)
	if err != nil {
		return nil, err
	}
	return cert.EncodeCertificates(candidateCert...)
}

// NormalizeTLSSecretCertChain validates a TLS key pair and normalizes the certificate chain.
// When only a root CA is present, no cert/key validation is performed.
func NormalizeTLSSecretCertChain(certChain, privateKey, rootCA []byte) ([]byte, error) {
	if len(certChain) == 0 && len(privateKey) == 0 && len(rootCA) != 0 {
		return certChain, nil
	}

	if err := ValidateTLSKeyPair(certChain, privateKey); err != nil {
		return nil, err
	}

	return NormalizePEMCerts(certChain)
}

// NormalizeCACerts validates and normalizes a CA certificate bundle.
func NormalizeCACerts(caCrtBytes []byte) ([]byte, error) {
	if len(caCrtBytes) == 0 {
		return nil, ErrMissingCACertKey
	}

	return NormalizePEMCerts(caCrtBytes)
}

// CACertsFromConfigMap extracts, validates, and normalizes ca.crt from a ConfigMap.
func CACertsFromConfigMap(cm *corev1.ConfigMap) ([]byte, error) {
	caCrt, ok := cm.Data["ca.crt"]
	if !ok {
		return nil, ErrMissingCACertKey
	}
	return NormalizeCACerts([]byte(caCrt))
}
