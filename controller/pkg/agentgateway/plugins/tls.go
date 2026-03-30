package plugins

import (
	"errors"
	"fmt"

	corev1 "k8s.io/api/core/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

var (
	ErrInvalidTlsSecret = errors.New("invalid TLS secret")

	InvalidTlsSecretError = func(n, ns string, err error) error {
		return fmt.Errorf("%w %s/%s: %v", ErrInvalidTlsSecret, ns, n, err)
	}

	ErrMissingCACertKey = utils.ErrMissingCACertKey

	ErrInvalidCACertificate = func(n, ns string, err error) error {
		return fmt.Errorf("invalid ca.crt in ConfigMap %s/%s: %v", ns, n, err)
	}
)

func ValidateTlsSecretData(n, ns string, sslSecretData map[string][]byte) (cleanedCertChain string, err error) {
	certChain := string(sslSecretData[corev1.TLSCertKey])
	privateKey := string(sslSecretData[corev1.TLSPrivateKeyKey])
	rootCa := string(sslSecretData[corev1.ServiceAccountRootCAKey])

	cleanedChainBytes, err := utils.NormalizeTLSSecretCertChain([]byte(certChain), []byte(privateKey), []byte(rootCa))
	if err != nil {
		err = InvalidTlsSecretError(n, ns, err)
		return "", err
	}
	return string(cleanedChainBytes), nil
}

// GetCACertFromConfigMap validates and extracts the ca.crt string from a ConfigMap
func GetCACertFromConfigMap(cm *corev1.ConfigMap) (string, error) {
	cleanedChainBytes, err := utils.CACertsFromConfigMap(cm)
	if err != nil {
		if errors.Is(err, ErrMissingCACertKey) {
			return "", ErrMissingCACertKey
		}
		return "", ErrInvalidCACertificate(cm.Name, cm.Namespace, err)
	}
	return string(cleanedChainBytes), nil
}
