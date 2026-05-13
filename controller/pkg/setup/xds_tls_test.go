package setup

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
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

func TestGenerateLeafFromCAIncludesXdsHosts(t *testing.T) {
	caCert, caKey, err := generateCA("test-ca")
	require.NoError(t, err)

	certPEM, _, err := generateLeafFromCA(caCert, caKey, []string{
		"agentgateway.agentgateway-system.svc.cluster.local",
		"10.0.0.1",
	})
	require.NoError(t, err)

	block, _ := pem.Decode(certPEM)
	require.NotNil(t, block)
	cert, err := x509.ParseCertificate(block.Bytes)
	require.NoError(t, err)
	require.NoError(t, cert.VerifyHostname("agentgateway.agentgateway-system.svc.cluster.local"))
	require.NoError(t, cert.VerifyHostname("10.0.0.1"))
}

func TestExtractServingMaterialUsesDirectServingCert(t *testing.T) {
	caCert, _, err := generateCA("test-ca")
	require.NoError(t, err)
	servingCert, servingKey := generateServingCert(t)
	secret := xdsSecret(map[string][]byte{
		xdsCertKey:   servingCert,
		xdsKeyKey:    servingKey,
		xdsCACertKey: caCert,
	})

	gotCert, gotKey, err := extractServingMaterial(secret, []string{"xds.default.svc"})
	require.NoError(t, err)
	require.Equal(t, servingCert, gotCert)
	require.Equal(t, servingKey, gotKey)
}

func TestExtractServingMaterialRejectsServingCertWithoutCA(t *testing.T) {
	servingCert, servingKey := generateServingCert(t)
	secret := xdsSecret(map[string][]byte{
		xdsCertKey: servingCert,
		xdsKeyKey:  servingKey,
	})

	_, _, err := extractServingMaterial(secret, []string{"xds.default.svc"})
	require.ErrorContains(t, err, "must include ca.crt")
}

func TestExtractServingMaterialRejectsPartialDirectCert(t *testing.T) {
	servingCert, _ := generateServingCert(t)
	secret := xdsSecret(map[string][]byte{
		xdsCertKey: servingCert,
	})

	_, _, err := extractServingMaterial(secret, []string{"xds.default.svc"})
	require.ErrorContains(t, err, "must include both tls.crt and tls.key")
}

func TestExtractServingMaterialUsesCertManagerStyleCASecret(t *testing.T) {
	caCert, caKey, err := generateCA("test-ca")
	require.NoError(t, err)
	secret := xdsSecret(map[string][]byte{
		xdsCertKey: caCert,
		xdsKeyKey:  caKey,
	})

	leafCertPEM, _, err := extractServingMaterial(secret, []string{"xds.default.svc"})
	require.NoError(t, err)

	leaf := parseTestCert(t, leafCertPEM)
	require.False(t, leaf.IsCA)
	require.NoError(t, leaf.VerifyHostname("xds.default.svc"))
	roots := x509.NewCertPool()
	require.True(t, roots.AppendCertsFromPEM(caCert))
	_, err = leaf.Verify(x509.VerifyOptions{
		DNSName: "xds.default.svc",
		Roots:   roots,
		KeyUsages: []x509.ExtKeyUsage{
			x509.ExtKeyUsageServerAuth,
		},
	})
	require.NoError(t, err)
}

func TestGenerateLeafFromCAAcceptsLegacyKeyEncodings(t *testing.T) {
	t.Run("rsa pkcs1", func(t *testing.T) {
		caCert, caKey := generateRSACA(t)
		_, _, err := generateLeafFromCA(caCert, caKey, []string{"xds.default.svc"})
		require.NoError(t, err)
	})
	t.Run("ec sec1", func(t *testing.T) {
		caCert, caKey := generateECCASEC1(t)
		_, _, err := generateLeafFromCA(caCert, caKey, []string{"xds.default.svc"})
		require.NoError(t, err)
	})
}

func TestGenerateLeafFromCARejectsMismatchedKey(t *testing.T) {
	caCert, _, err := generateCA("test-ca")
	require.NoError(t, err)
	_, otherKey, err := generateCA("other-ca")
	require.NoError(t, err)

	_, _, err = generateLeafFromCA(caCert, otherKey, []string{"xds.default.svc"})
	require.ErrorContains(t, err, "do not match")
}

func TestGenerateCAUsesECDSAAndTenYearLifetime(t *testing.T) {
	caCertPEM, caKeyPEM, err := generateCA("test-ca")
	require.NoError(t, err)

	certBlock, _ := pem.Decode(caCertPEM)
	require.NotNil(t, certBlock)
	cert, err := x509.ParseCertificate(certBlock.Bytes)
	require.NoError(t, err)
	require.IsType(t, &ecdsa.PublicKey{}, cert.PublicKey)
	require.WithinDuration(t, time.Now().Add(xdsCACertLifetime), cert.NotAfter, time.Minute)

	keyBlock, _ := pem.Decode(caKeyPEM)
	require.NotNil(t, keyBlock)
	key, err := x509.ParsePKCS8PrivateKey(keyBlock.Bytes)
	require.NoError(t, err)
	require.IsType(t, &ecdsa.PrivateKey{}, key)
}

func TestShouldRefreshXdsTLSMaterialForGeneratedCerts(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:            "xds",
			Namespace:       "default",
			ResourceVersion: "1",
		},
		Data: map[string][]byte{
			xdsCACertKey: []byte("ca"),
			xdsCAKeyKey:  []byte("key"),
		},
	}
	material := &xdsTLSMaterial{}

	require.True(t, material.shouldRefresh(secret, "1"))

	material.setCertificate(testCertificate(t, time.Now().Add(xdsLeafCertRenewBefore+time.Hour)))
	require.False(t, material.shouldRefresh(secret, "1"))

	material.setCertificate(testCertificate(t, time.Now().Add(time.Hour)))
	require.True(t, material.shouldRefresh(secret, "1"))
}

func TestShouldRefreshXdsTLSMaterialForCertManagerStyleCASecret(t *testing.T) {
	caCert, caKey, err := generateCA("test-ca")
	require.NoError(t, err)
	secret := xdsSecret(map[string][]byte{
		xdsCertKey: caCert,
		xdsKeyKey:  caKey,
	})
	secret.ResourceVersion = "1"
	material := &xdsTLSMaterial{}

	require.True(t, material.shouldRefresh(secret, "1"))

	material.setCertificate(testCertificate(t, time.Now().Add(xdsLeafCertRenewBefore+time.Hour)))
	require.False(t, material.shouldRefresh(secret, "1"))

	material.setCertificate(testCertificate(t, time.Now().Add(time.Hour)))
	require.True(t, material.shouldRefresh(secret, "1"))
}

func TestShouldRefreshXdsTLSMaterialForDirectServingCertOnlyOnResourceVersion(t *testing.T) {
	caCert, _, err := generateCA("test-ca")
	require.NoError(t, err)
	servingCert, servingKey := generateServingCert(t)
	secret := xdsSecret(map[string][]byte{
		xdsCertKey:   servingCert,
		xdsKeyKey:    servingKey,
		xdsCACertKey: caCert,
	})
	secret.ResourceVersion = "1"
	material := &xdsTLSMaterial{}
	material.setCertificate(testCertificate(t, time.Now().Add(time.Hour)))

	require.False(t, material.shouldRefresh(secret, "1"))
	require.True(t, material.shouldRefresh(secret, "2"))
}

func TestXdsTLSMaterialSyncerRefreshesChangedSecret(t *testing.T) {
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	material := &xdsTLSMaterial{}
	s := &xdsTLSMaterialSyncer{
		ctx:      ctx,
		hosts:    []string{"xds.default.svc"},
		material: material,
	}

	initialCert, initialKey, err := generateCA("initial-ca")
	require.NoError(t, err)
	initialSecret := xdsSecret(map[string][]byte{
		xdsCertKey: initialCert,
		xdsKeyKey:  initialKey,
	})
	initialSecret.ResourceVersion = "1"
	require.NoError(t, s.refreshSecret(initialSecret, true))

	cert, err := material.GetCertificate(nil)
	require.NoError(t, err)
	require.Equal(t, "initial-ca", cert.Leaf.Issuer.CommonName)

	caCert, caKey, err := generateCA("watch-ca")
	require.NoError(t, err)
	updatedSecret := xdsSecret(map[string][]byte{
		xdsCertKey: caCert,
		xdsKeyKey:  caKey,
	})
	updatedSecret.ResourceVersion = "2"

	require.NoError(t, s.refreshSecret(updatedSecret, false))
	cert, err = material.GetCertificate(nil)
	require.NoError(t, err)
	require.Equal(t, "watch-ca", cert.Leaf.Issuer.CommonName)
}

func testCertificate(t *testing.T, notAfter time.Time) tls.Certificate {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	require.NoError(t, err)
	serial, err := rand.Int(rand.Reader, big.NewInt(1<<62))
	require.NoError(t, err)
	tpl := &x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: "test"},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     notAfter,
	}
	der, err := x509.CreateCertificate(rand.Reader, tpl, tpl, &key.PublicKey, key)
	require.NoError(t, err)
	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	keyDER, err := x509.MarshalPKCS8PrivateKey(key)
	require.NoError(t, err)
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER})
	cert, err := tls.X509KeyPair(certPEM, keyPEM)
	require.NoError(t, err)
	cert.Leaf, err = x509.ParseCertificate(der)
	require.NoError(t, err)
	return cert
}

func xdsSecret(data map[string][]byte) *corev1.Secret {
	return &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "xds",
			Namespace: "default",
		},
		Data: data,
	}
}

func parseTestCert(t *testing.T, certPEM []byte) *x509.Certificate {
	t.Helper()
	cert, err := parseCertificate(certPEM)
	require.NoError(t, err)
	return cert
}

func generateServingCert(t *testing.T) ([]byte, []byte) {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	require.NoError(t, err)
	serial, err := rand.Int(rand.Reader, big.NewInt(1<<62))
	require.NoError(t, err)
	tpl := &x509.Certificate{
		SerialNumber: serial,
		Subject:      pkix.Name{CommonName: "xds.default.svc"},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature,
		ExtKeyUsage:  []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		DNSNames:     []string{"xds.default.svc"},
	}
	der, err := x509.CreateCertificate(rand.Reader, tpl, tpl, &key.PublicKey, key)
	require.NoError(t, err)
	keyPEM, err := encodePrivateKey(key)
	require.NoError(t, err)
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), keyPEM
}

func generateRSACA(t *testing.T) ([]byte, []byte) {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	require.NoError(t, err)
	cert := generateCACert(t, "rsa-ca", &key.PublicKey, key)
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)})
	return cert, keyPEM
}

func generateECCASEC1(t *testing.T) ([]byte, []byte) {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	require.NoError(t, err)
	cert := generateCACert(t, "ec-ca", &key.PublicKey, key)
	keyDER, err := x509.MarshalECPrivateKey(key)
	require.NoError(t, err)
	keyPEM := pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: keyDER})
	return cert, keyPEM
}

func generateCACert(t *testing.T, commonName string, publicKey, privateKey any) []byte {
	t.Helper()
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
	der, err := x509.CreateCertificate(rand.Reader, tpl, tpl, publicKey, privateKey)
	require.NoError(t, err)
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
}
