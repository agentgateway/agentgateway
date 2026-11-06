package plugins

import (
	"crypto/fips140"
	"crypto/tls"
	"errors"
	"fmt"
	"strings"

	corev1 "k8s.io/api/core/v1"
	"k8s.io/client-go/util/cert"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func validatePolicyFIPSTLSOptions(policy *agentgateway.AgentgatewayPolicy) *ConfigError {
	if !fips140.Enabled() {
		return nil
	}

	var problems []string
	if frontend := policy.Spec.Frontend; frontend != nil && frontend.TLS != nil {
		if err := validateFIPSTLSOptions(frontend.TLS.CipherSuites, frontend.TLS.KeyExchangeGroups); err != nil {
			problems = append(problems, fmt.Sprintf("frontend.tls: %v", err))
		}
	}
	if backend := policy.Spec.Backend; backend != nil && backend.TLS != nil {
		if err := validateFIPSTLSOptions(nil, backend.TLS.KeyExchangeGroups); err != nil {
			problems = append(problems, fmt.Sprintf("backend.tls: %v", err))
		}
	}
	if len(problems) > 0 {
		return &ConfigError{Reason: agentgateway.PolicyReasonInvalid, Message: strings.Join(problems, "; ")}
	}
	return nil
}

func validateFIPSTLSOptions(cipherSuites []agentgateway.CipherSuite, keyExchangeGroups []agentgateway.KeyExchangeGroup) error {
	// Keep in sync with provider_with_options_validated in crates/agentgateway/src/crypto/tls.rs.
	var badSuites []agentgateway.CipherSuite
	var badGroups []agentgateway.KeyExchangeGroup
	for _, suite := range cipherSuites {
		switch suite {
		case agentgateway.CipherSuiteTLS13_CHACHA20_POLY1305_SHA256,
			agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
			agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256:
			badSuites = append(badSuites, suite)
		}
	}
	for _, group := range keyExchangeGroups {
		if group == agentgateway.KeyExchangeGroupX25519 {
			badGroups = append(badGroups, group)
		}
	}
	if len(badSuites) == 0 && len(badGroups) == 0 {
		return nil
	}
	return fmt.Errorf("TLS configuration is not permitted in FIPS mode: non-approved cipher suites %v, non-approved key exchange groups %v", badSuites, badGroups)
}

var (
	ErrInvalidTlsSecret = errors.New("invalid TLS secret")

	InvalidTlsSecretError = func(n, ns string, err error) error {
		return fmt.Errorf("%w %s/%s: %v", ErrInvalidTlsSecret, ns, n, err)
	}

	ErrMissingCACertKey = errors.New("ca.crt key missing")

	ErrInvalidCACertificate = func(n, ns string, err error) error {
		return fmt.Errorf("invalid ca.crt in ConfigMap %s/%s: %v", ns, n, err)
	}
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
