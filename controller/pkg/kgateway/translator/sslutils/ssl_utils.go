package sslutils

import (
	"crypto/tls"
	"errors"
	"fmt"

	corev1 "k8s.io/api/core/v1"
	"k8s.io/client-go/util/cert"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

// Temporary home for constants for conformance testing that are not yet in a released version of Gateway API (https://github.com/kubernetes-sigs/gateway-api/blob/aa1ab6fd282dee4f74eeca803ec48b333297c637/apis/v1/gateway_types.go#L1606-L1614)
const (
	ListenerReasonInvalidCACertificateRef  gwv1.ListenerConditionReason = "InvalidCACertificateRef"
	ListenerReasonInvalidCACertificateKind gwv1.ListenerConditionReason = "InvalidCACertificateKind"
	ListenerReasonNoValidCACertificate     gwv1.ListenerConditionReason = "NoValidCACertificate"
)

var (
	ErrInvalidTlsSecret = errors.New("invalid TLS secret")

	InvalidTlsSecretError = func(n, ns string, err error) error {
		return fmt.Errorf("%w %s/%s: %v", ErrInvalidTlsSecret, ns, n, err)
	}

	ErrMissingCACertKey = errors.New("ca.crt key missing")

	ErrInvalidCACertificate = func(n, ns string, err error) error {
		return fmt.Errorf("invalid ca.crt in ConfigMap %s/%s: %v", ns, n, err)
	}

	ErrVerifySubjectAltNamesRequiresCA = errors.New("verify-subject-alt-names annotation requires a trusted CA to be configured")

	ErrInvalidCACertificateRef  = errors.New(string(ListenerReasonInvalidCACertificateRef))
	ErrInvalidCACertificateKind = errors.New(string(ListenerReasonInvalidCACertificateKind))

	ErrInvalidCACertificateKindDetails = func(n, ns, kind string) error {
		return fmt.Errorf("invalid ca.crt kind %s in %s/%s: %w", kind, ns, n, ErrInvalidCACertificateKind)
	}
	ErrMissingCaCertificateRefGrant = errors.New("missing CA certificate reference grant")
)

func ValidateTlsSecretData(n, ns string, sslSecretData map[string][]byte) (cleanedCertChain string, err error) {
	certChain := string(sslSecretData[corev1.TLSCertKey])
	privateKey := string(sslSecretData[corev1.TLSPrivateKeyKey])
	rootCa := string(sslSecretData[corev1.ServiceAccountRootCAKey])

	cleanedCertChain, err = cleanedSslKeyPair(certChain, privateKey, rootCa)
	if err != nil {
		err = InvalidTlsSecretError(n, ns, err)
	}
	return cleanedCertChain, err
}

func cleanedSslKeyPair(certChain, privateKey, rootCa string) (cleanedChain string, err error) {
	// in the case where we _only_ provide a rootCa, we do not want to validate tls.key+tls.cert
	if (certChain == "") && (privateKey == "") && (rootCa != "") {
		return certChain, nil
	}

	// validate that the cert and key are a valid pair
	_, err = tls.X509KeyPair([]byte(certChain), []byte(privateKey))
	if err != nil {
		return "", err
	}

	// validate that the parsed piece is valid
	// this is still faster than a call out to openssl despite this second parsing pass of the cert
	// pem parsing in go is permissive while envoy is not
	// this might not be needed once we have larger envoy validation
	candidateCert, err := cert.ParseCertsPEM([]byte(certChain))
	if err != nil {
		// return err rather than sanitize. This is to maintain UX with older versions and to keep in line with kgateway pkg.
		return "", err
	}
	cleanedChainBytes, err := cert.EncodeCertificates(candidateCert...)
	cleanedChain = string(cleanedChainBytes)

	return cleanedChain, err
}

// GetCACertFromConfigMap validates and extracts the ca.crt string from a ConfigMap
func GetCACertFromConfigMap(cm *corev1.ConfigMap) (string, error) {
	caCrt, ok := cm.Data["ca.crt"]
	if !ok {
		return "", ErrMissingCACertKey
	}
	return getCACertFromBytes([]byte(caCrt), cm.Name, cm.Namespace)
}

// getCACertFromBytes validates and extracts the ca.crt string from certificate bytes
func getCACertFromBytes(caCrtBytes []byte, name, namespace string) (string, error) {
	if len(caCrtBytes) == 0 {
		return "", ErrMissingCACertKey
	}

	// Validate CA certificate by trying to parse it
	candidateCert, err := cert.ParseCertsPEM(caCrtBytes)
	if err != nil {
		return "", ErrInvalidCACertificate(name, namespace, err)
	}

	// Clean and encode the certificate to ensure proper formatting
	cleanedChainBytes, err := cert.EncodeCertificates(candidateCert...)
	if err != nil {
		return "", ErrInvalidCACertificate(name, namespace, err)
	}

	return string(cleanedChainBytes), nil
}
