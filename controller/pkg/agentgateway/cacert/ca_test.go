package cacert_test

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/pem"
	"math/big"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/cacert"
)

// TestResolveKinds covers selecting the ConfigMap or Secret a CA bundle is read from.
func TestResolveKinds(t *testing.T) {
	const namespace = "default"
	configMapCA := testCAPEM(t, 1)
	secretCA := testCAPEM(t, 2)

	tests := []struct {
		name      string
		ref       agentgateway.LocalCACertificateRef
		configMap *corev1.ConfigMap
		secret    *corev1.Secret
		want      []byte
		wantErr   string
	}{
		{
			name:      "ConfigMap",
			ref:       agentgateway.LocalCACertificateRef{Kind: "ConfigMap", Name: "ca"},
			configMap: &corev1.ConfigMap{Name: "ca", Namespace: namespace, Data: map[string]string{"ca.crt": string(configMapCA)}},
			want:      configMapCA,
		},
		{
			name:   "Secret",
			ref:    agentgateway.LocalCACertificateRef{Kind: "Secret", Name: "ca"},
			secret: &corev1.Secret{Name: "ca", Namespace: namespace, Data: map[string][]byte{"ca.crt": secretCA}},
			want:   secretCA,
		},
		{
			name:      "omitted kind defaults to ConfigMap",
			ref:       agentgateway.LocalCACertificateRef{Name: "ca"},
			configMap: &corev1.ConfigMap{Name: "ca", Namespace: namespace, Data: map[string]string{"ca.crt": string(configMapCA)}},
			want:      configMapCA,
		},
		{
			name:      "Secret ref does not fall back to a same-name ConfigMap",
			ref:       agentgateway.LocalCACertificateRef{Kind: "Secret", Name: "ca"},
			configMap: &corev1.ConfigMap{Name: "ca", Namespace: namespace, Data: map[string]string{"ca.crt": string(configMapCA)}},
			wantErr:   "Secret default/ca not found",
		},
		{
			name:    "missing ConfigMap",
			ref:     agentgateway.LocalCACertificateRef{Kind: "ConfigMap", Name: "ca"},
			wantErr: "ConfigMap default/ca not found",
		},
		{
			name:    "Secret missing ca.crt",
			ref:     agentgateway.LocalCACertificateRef{Kind: "Secret", Name: "ca"},
			secret:  &corev1.Secret{Name: "ca", Namespace: namespace},
			wantErr: `missing key "ca.crt"`,
		},
		{
			name:    "Secret with invalid PEM",
			ref:     agentgateway.LocalCACertificateRef{Kind: "Secret", Name: "ca"},
			secret:  &corev1.Secret{Name: "ca", Namespace: namespace, Data: map[string][]byte{"ca.crt": []byte("not pem")}},
			wantErr: `invalid CA certificate in Secret default/ca key "ca.crt"`,
		},
		{
			name:    "unsupported kind",
			ref:     agentgateway.LocalCACertificateRef{Kind: "Service", Name: "ca"},
			wantErr: `unsupported CA certificate reference kind "Service"`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := resolve(t, namespace, tt.ref, tt.configMap, tt.secret)

			if tt.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("error = %v, want substring %q", err, tt.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if got != string(tt.want) {
				t.Fatalf("CA = %q, want %q", got, tt.want)
			}
		})
	}
}

// TestResolveKey covers selecting the data key the CA bundle is read from.
func TestResolveKey(t *testing.T) {
	const namespace = "default"
	defaultCA := testCAPEM(t, 1)
	customCA := testCAPEM(t, 2)

	// One object carrying two distinct bundles: only a key selector can reach the second.
	configMap := &corev1.ConfigMap{Name: "ca", Namespace: namespace, Data: map[string]string{
		"ca.crt":              string(defaultCA),
		"corporate-roots.pem": string(customCA),
	}}
	secret := &corev1.Secret{Name: "ca", Namespace: namespace, Data: map[string][]byte{
		"ca.crt":              defaultCA,
		"corporate-roots.pem": customCA,
	}}

	tests := []struct {
		name    string
		ref     agentgateway.LocalCACertificateRef
		want    []byte
		wantErr string
	}{
		{
			name: "ConfigMap omitted key defaults to ca.crt",
			ref:  agentgateway.LocalCACertificateRef{Kind: "ConfigMap", Name: "ca"},
			want: defaultCA,
		},
		{
			name: "ConfigMap explicit ca.crt matches the default",
			ref:  agentgateway.LocalCACertificateRef{Kind: "ConfigMap", Name: "ca", Key: "ca.crt"},
			want: defaultCA,
		},
		{
			name: "ConfigMap custom key",
			ref:  agentgateway.LocalCACertificateRef{Kind: "ConfigMap", Name: "ca", Key: "corporate-roots.pem"},
			want: customCA,
		},
		{
			name: "Secret omitted key defaults to ca.crt",
			ref:  agentgateway.LocalCACertificateRef{Kind: "Secret", Name: "ca"},
			want: defaultCA,
		},
		{
			name: "Secret custom key",
			ref:  agentgateway.LocalCACertificateRef{Kind: "Secret", Name: "ca", Key: "corporate-roots.pem"},
			want: customCA,
		},
		{
			name:    "ConfigMap missing custom key names the key",
			ref:     agentgateway.LocalCACertificateRef{Kind: "ConfigMap", Name: "ca", Key: "absent.pem"},
			wantErr: `error extracting CA cert from ConfigMap default/ca: missing key "absent.pem"`,
		},
		{
			name:    "Secret missing custom key names the key",
			ref:     agentgateway.LocalCACertificateRef{Kind: "Secret", Name: "ca", Key: "absent.pem"},
			wantErr: `error extracting CA cert from Secret default/ca: missing key "absent.pem"`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := resolve(t, namespace, tt.ref, configMap, secret)

			if tt.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("error = %v, want substring %q", err, tt.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if got != string(tt.want) {
				t.Fatalf("CA = %q, want %q", got, tt.want)
			}
		})
	}
}

func resolve(
	t *testing.T,
	namespace string,
	ref agentgateway.LocalCACertificateRef,
	configMap *corev1.ConfigMap,
	secret *corev1.Secret,
) (string, error) {
	t.Helper()
	var configMaps []*corev1.ConfigMap
	if configMap != nil {
		configMaps = []*corev1.ConfigMap{configMap}
	}
	var secrets []*corev1.Secret
	if secret != nil {
		secrets = []*corev1.Secret{secret}
	}
	return cacert.Resolve(
		krt.TestingDummyContext{},
		krt.NewStaticCollection(nil, configMaps, krt.WithName("cacert/ConfigMaps")),
		krt.NewStaticCollection(nil, secrets, krt.WithName("cacert/Secrets")),
		namespace,
		ref,
	)
}

func testCAPEM(t *testing.T, serial int64) []byte {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(serial),
		NotBefore:             now,
		NotAfter:              now.Add(time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}
	certificate, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificate})
}
