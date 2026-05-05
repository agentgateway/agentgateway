package deployer

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

func TestExtractXdsCACertificatePrefersCACrt(t *testing.T) {
	caCert, _ := generateTestCA(t, "ca-crt")
	otherCert, otherKey := generateTestCA(t, "tls-crt")
	secret := xdsTLSSecret(map[string][]byte{
		corev1.ServiceAccountRootCAKey: caCert,
		corev1.TLSCertKey:              otherCert,
		corev1.TLSPrivateKeyKey:        otherKey,
	})

	got, err := extractXdsCACertificate(secret)
	require.NoError(t, err)
	require.Equal(t, string(caCert), got)
}

func TestExtractXdsCACertificateUsesCertManagerStyleCASecret(t *testing.T) {
	caCert, caKey := generateTestCA(t, "tls-ca")
	secret := xdsTLSSecret(map[string][]byte{
		corev1.TLSCertKey:       caCert,
		corev1.TLSPrivateKeyKey: caKey,
	})

	got, err := extractXdsCACertificate(secret)
	require.NoError(t, err)
	require.Equal(t, string(caCert), got)
}

func xdsTLSSecret(data map[string][]byte) *corev1.Secret {
	return &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "xds",
			Namespace: "default",
		},
		Data: data,
	}
}

func generateTestCA(t *testing.T, commonName string) ([]byte, []byte) {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	require.NoError(t, err)
	serial, err := rand.Int(rand.Reader, big.NewInt(1<<62))
	require.NoError(t, err)
	tpl := &x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: commonName},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),

		IsCA:                  true,
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
		BasicConstraintsValid: true,
	}
	der, err := x509.CreateCertificate(rand.Reader, tpl, tpl, &key.PublicKey, key)
	require.NoError(t, err)
	keyDER, err := x509.MarshalPKCS8PrivateKey(key)
	require.NoError(t, err)
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}),
		pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER})
}
