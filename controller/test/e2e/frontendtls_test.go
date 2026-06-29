//go:build e2e

package e2e_test

import (
	"crypto/rand"
	"crypto/rsa"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
)

const (
	frontendTLSNamespace = "agentgateway-frontendtls"
	frontendTLSHostname  = "test.example.com"
)

func TestFrontendTLS(tt *testing.T) {
	t := New(tt, base.WithMinGwApiVersion(base.GwApiRequireFrontendTLSConfig))
	ca1, ca1Key := generateCA(t, "*", 24*time.Hour, nil, nil)
	ca2, ca2Key := generateCA(t, "*", 24*time.Hour, nil, nil)

	client1, client1Key := generateCertificate(t, "client1.example.com", 24*time.Hour, ca1, ca1Key)
	client2, client2Key := generateCertificate(t, "client2.example.com", 24*time.Hour, ca2, ca2Key)

	server, serverKey := generateCertificate(t, "*.example.com", 24*time.Hour, ca1, ca1Key)

	t.Apply(
		manifest("frontendtls", "namespace.yaml"),
		manifest("frontendtls", "routes.yaml"),
	)
	t.ApplyYAML(
		caManifest(t, "ca1", certificatePEM(ca1)),
		caManifest(t, "ca2", certificatePEM(ca2)),
		caManifest(t, "invalid1", []byte("invalid1")),
		tlsSecretManifest(t, "gateway-cert", ca1, server, serverKey),
	)

	t.Run("ClientCertValidation", func(t base.Test) {
		t.Apply(manifest("frontendtls", "gateway-ca1.yaml"))
		gateway := gateway(t)
		assertSuccess(t, gateway, client1, client1Key, ca1)
		assertFailure(t, gateway, client2, client2Key, ca1)
	})
	t.Run("ClientCertValidationAllowInsecureFallback", func(t base.Test) {
		t.Apply(manifest("frontendtls", "gateway-ca1-with-insecure-fallback.yaml"))
		gateway := gateway(t)
		assertSuccess(t, gateway, client1, client1Key, ca1)
		assertSuccess(t, gateway, client2, client2Key, ca1)
	})
	t.Run("ClientCertValidationWithMultipleCAs", func(t base.Test) {
		t.Apply(manifest("frontendtls", "gateway-ca1-ca2.yaml"))
		gateway := gateway(t)
		assertSuccess(t, gateway, client1, client1Key, ca1)
		assertSuccess(t, gateway, client2, client2Key, ca1)
	})
	t.Run("ClientCertValidationWithSomeCARefsInvalid", func(t base.Test) {
		t.Apply(manifest("frontendtls", "gateway-ca1-invalid1.yaml"))
		gateway := gateway(t)
		assertListenerConditions(t, map[gwv1.ListenerConditionType]metav1.ConditionStatus{
			gwv1.ListenerConditionAccepted:     metav1.ConditionTrue,
			gwv1.ListenerConditionProgrammed:   metav1.ConditionTrue,
			gwv1.ListenerConditionResolvedRefs: metav1.ConditionFalse,
		})
		assertSuccess(t, gateway, client1, client1Key, ca1)
		assertFailure(t, gateway, client2, client2Key, ca1)
	})
	t.Run("ClientCertValidationWithAllCARefsInvalid", func(t base.Test) {
		t.Apply(manifest("frontendtls", "gateway-invalid1-invalid2.yaml"))
		assertListenerConditions(t, map[gwv1.ListenerConditionType]metav1.ConditionStatus{
			gwv1.ListenerConditionAccepted:     metav1.ConditionFalse,
			gwv1.ListenerConditionProgrammed:   metav1.ConditionFalse,
			gwv1.ListenerConditionResolvedRefs: metav1.ConditionFalse,
		})
		gateway := gateway(t)
		assertFailure(t, gateway, client1, client1Key, ca1)
		assertFailure(t, gateway, client2, client2Key, ca1)
	})
}

func assertSuccess(t base.Test, gateway base.Gateway, clientCert *x509.Certificate, clientKey *rsa.PrivateKey, cas ...*x509.Certificate) {
	caCertPool := x509.NewCertPool()
	for _, ca := range cas {
		caCertPool.AddCert(ca)
	}

	cert, err := tls.X509KeyPair(certificatePEM(clientCert), privateKeyPEM(t, clientKey))
	assert.NoError(t, err)

	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{cert},
		RootCAs:      caCertPool,
		// Generating certs with proper gateway IP SANs is bothersome and not the point of this test,
		// so we skip verification of the server certs on the client side.
		//gosec:disable G402
		InsecureSkipVerify: true,
	}

	opts := []curl.Option{
		curl.WithPort(gateway.PortForRemote(443)),
		curl.WithScheme("https"),
		curl.WithHostHeader(frontendTLSHostname),
		curl.WithTLSConfig(tlsConfig),
		curl.WithPath("/"),
	}

	gateway.Send(t, base.ExpectOK(), opts...)
}

func assertFailure(t base.Test, gateway base.Gateway, clientCert *x509.Certificate, clientKey *rsa.PrivateKey, cas ...*x509.Certificate) {
	caCertPool := x509.NewCertPool()
	for _, ca := range cas {
		caCertPool.AddCert(ca)
	}

	cert, err := tls.X509KeyPair(certificatePEM(clientCert), privateKeyPEM(t, clientKey))
	assert.NoError(t, err)

	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{cert},
		RootCAs:      caCertPool,
		// Generating certs with proper gateway IP SANs is bothersome and not the point of this test,
		// so we skip verification of the server certs on the client side.
		//gosec:disable G402
		InsecureSkipVerify: true,
	}

	addr := gateway.ResolvedAddress()
	opts := append(base.GatewayAddressOptions(addr),
		curl.WithPort(gateway.PortForRemote(443)),
		curl.WithScheme("https"),
		curl.WithHostHeader(frontendTLSHostname),
		curl.WithTLSConfig(tlsConfig),
		curl.WithPath("/"),
	)

	connectionError := fmt.Errorf("failed to connect to gateway %s/%s (%s)", gateway.Namespace, gateway.Name, addr)
	assert.Consistently(t, func() error {
		r, err := curl.ExecuteRequest(opts...)
		if err != nil {
			return connectionError
		}
		r.Body.Close()
		return nil
	}, connectionError, 10*time.Second)
}

func gateway(t base.Test) base.Gateway {
	t.GatewayReady("gateway", frontendTLSNamespace)
	assertions.EventuallyGatewayListenerAttachedRoutes(t,
		"gateway",
		frontendTLSNamespace,
		gwv1.SectionName("https"),
		int32(1),
	)
	name := types.NamespacedName{Name: "gateway", Namespace: frontendTLSNamespace}
	return base.Gateway{
		NamespacedName: name,
		Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, name),
	}
}

func assertListenerConditions(t base.Test, expected map[gwv1.ListenerConditionType]metav1.ConditionStatus) {
	for conditionType, expectedStatus := range expected {
		assertions.EventuallyGatewayListenerCondition(
			t,
			"gateway",
			frontendTLSNamespace,
			gwv1.SectionName("https"),
			conditionType,
			expectedStatus,
		)
	}
}

func tlsSecretManifest(t base.Test, name string, ca, cert *x509.Certificate, key *rsa.PrivateKey) string {
	return fmt.Sprintf(`
apiVersion: v1
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
		name,
		frontendTLSNamespace,
		base64.StdEncoding.EncodeToString(certificatePEM(ca)),
		base64.StdEncoding.EncodeToString(certificatePEM(cert)),
		base64.StdEncoding.EncodeToString(privateKeyPEM(t, key)),
	)
}

func caManifest(t base.Test, name string, cert []byte) string {
	cm := &corev1.ConfigMap{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "v1",
			Kind:       "ConfigMap",
		},
		ObjectMeta: metav1.ObjectMeta{
			Namespace: frontendTLSNamespace,
			Name:      name,
		},
		Data: map[string]string{
			"ca.crt": string(cert),
		},
	}
	data, err := json.Marshal(cm)
	assert.NoError(t, err)
	return string(data)
}

func certificatePEM(cert *x509.Certificate) []byte {
	return pem.EncodeToMemory(&pem.Block{
		Type:  "CERTIFICATE",
		Bytes: cert.Raw,
	})
}

func privateKeyPEM(t base.Test, key *rsa.PrivateKey) []byte {
	bytes, err := x509.MarshalPKCS8PrivateKey(key)
	assert.NoError(t, err)
	return pem.EncodeToMemory(&pem.Block{
		Type:  "PRIVATE KEY",
		Bytes: bytes,
	})
}

func generateCA(t base.Test, commonName string, validFor time.Duration, parent *x509.Certificate, parentKey *rsa.PrivateKey) (*x509.Certificate, *rsa.PrivateKey) {
	if (parent != nil) != (parentKey != nil) {
		t.Fatal("both parent certificate and parent private key must be provided together")
	}

	notBefore := time.Now()
	notAfter := notBefore.Add(validFor)

	template := &x509.Certificate{
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

	key, err := rsa.GenerateKey(rand.Reader, 2048)
	assert.NoError(t, err)

	if parent != nil {
		der, err := x509.CreateCertificate(rand.Reader, template, parent, &key.PublicKey, parentKey)
		assert.NoError(t, err)
		return certificate(t, der), key
	}

	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	assert.NoError(t, err)
	return certificate(t, der), key
}

func generateCertificate(t base.Test, commonName string, validFor time.Duration, parent *x509.Certificate, parentKey *rsa.PrivateKey) (*x509.Certificate, *rsa.PrivateKey) {
	if parent == nil || parentKey == nil {
		t.Fatal("both signing CA certificate and private key must be provided")
	}

	notBefore := time.Now()
	notAfter := notBefore.Add(validFor)
	template := &x509.Certificate{
		Subject: pkix.Name{
			CommonName: commonName,
		},
		NotBefore:             notBefore,
		NotAfter:              notAfter,
		KeyUsage:              x509.KeyUsageKeyEncipherment | x509.KeyUsageDigitalSignature,
		ExtKeyUsage:           []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth, x509.ExtKeyUsageClientAuth},
		BasicConstraintsValid: true,
		IsCA:                  false,
	}

	key, err := rsa.GenerateKey(rand.Reader, 2048)
	assert.NoError(t, err)

	der, err := x509.CreateCertificate(rand.Reader, template, parent, &key.PublicKey, parentKey)
	assert.NoError(t, err)
	return certificate(t, der), key
}

func certificate(t base.Test, derBytes []byte) *x509.Certificate {
	cert, err := x509.ParseCertificate(derBytes)
	assert.NoError(t, err)
	return cert
}
