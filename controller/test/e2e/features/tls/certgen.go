//go:build e2e

package tls

import (
	"bytes"
	"crypto/rand"
	"crypto/rsa"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/pem"
	"fmt"
	"math/big"
	"time"

	"github.com/kgateway-dev/kgateway/v2/api/settings"
)

const (
	// DefaultExpiration is the default expiration time for the certificate. Set
	// to 1 year.
	DefaultExpiration = 365 * 24 * time.Hour
)

// SecretManifest generates a dynamically generated self-signed certificate for testing
// and returns the secret manifest in YAML format.
func SecretManifest(ns string, expiration time.Duration) (string, error) {
	certPEM, keyPEM, err := generateSelfSignedCert("test-ca", expiration)
	if err != nil {
		return "", fmt.Errorf("failed to generate self-signed certificate: %v", err)
	}
	secretYAML := fmt.Sprintf(`apiVersion: v1
kind: Secret
metadata:
  name: %s
  namespace: %s
type: kubernetes.io/tls
data:
  ca.crt: %s
  tls.crt: %s
  tls.key: %s
`,
		settings.TLSSecretName,
		ns,
		base64.StdEncoding.EncodeToString(certPEM),
		base64.StdEncoding.EncodeToString(certPEM),
		base64.StdEncoding.EncodeToString(keyPEM),
	)
	return secretYAML, nil
}

// generateSelfSignedCert generates a self-signed certificate for testing.
// Returns PEM-encoded certificate and private key.
func generateSelfSignedCert(commonName string, validFor time.Duration) (certPEM, keyPEM []byte, err error) {
	serialNumberLimit := new(big.Int).Lsh(big.NewInt(1), 128)
	serialNumber, err := rand.Int(rand.Reader, serialNumberLimit)
	if err != nil {
		return nil, nil, fmt.Errorf("failed to generate serial number: %w", err)
	}

	notBefore := time.Now()
	notAfter := notBefore.Add(validFor)
	template := x509.Certificate{
		SerialNumber: serialNumber,
		Subject: pkix.Name{
			CommonName: commonName,
		},
		NotBefore:             notBefore,
		NotAfter:              notAfter,
		KeyUsage:              x509.KeyUsageKeyEncipherment | x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		BasicConstraintsValid: true,
		IsCA:                  true,
	}

	privateKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		return nil, nil, fmt.Errorf("failed to generate private key: %w", err)
	}
	derBytes, err := x509.CreateCertificate(rand.Reader, &template, &template, &privateKey.PublicKey, privateKey)
	if err != nil {
		return nil, nil, fmt.Errorf("failed to create certificate: %w", err)
	}
	certBuf := new(bytes.Buffer)
	if err := pem.Encode(certBuf, &pem.Block{Type: "CERTIFICATE", Bytes: derBytes}); err != nil {
		return nil, nil, fmt.Errorf("failed to encode certificate: %w", err)
	}

	keyBuf := new(bytes.Buffer)
	privBytes, err := x509.MarshalPKCS8PrivateKey(privateKey)
	if err != nil {
		return nil, nil, fmt.Errorf("failed to marshal private key: %w", err)
	}
	if err := pem.Encode(keyBuf, &pem.Block{Type: "PRIVATE KEY", Bytes: privBytes}); err != nil {
		return nil, nil, fmt.Errorf("failed to encode private key: %w", err)
	}

	return certBuf.Bytes(), keyBuf.Bytes(), nil
}
