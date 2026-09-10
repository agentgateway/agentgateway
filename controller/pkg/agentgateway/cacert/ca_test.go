package cacert_test

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/pem"
	"errors"
	"math/big"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/cacert"
)

// TestResolveGatewayRef covers the upstream Gateway API entry point used by BackendTLSPolicy and
// XBackend. A ConfigMap has "Core" support upstream; a Secret is an implementation-specific
// extension upstream explicitly permits.
func TestResolveGatewayRef(t *testing.T) {
	const namespace = "default"
	configMapCA := testCAPEM(t, 1)
	secretCA := testCAPEM(t, 2)

	tests := []struct {
		name      string
		ref       gwv1.LocalObjectReference
		configMap *corev1.ConfigMap
		secret    *corev1.Secret
		want      []byte
		wantErr   string
		wantIs    error
	}{
		{
			name:      "ConfigMap",
			ref:       gwv1.LocalObjectReference{Group: "", Kind: "ConfigMap", Name: "ca"},
			configMap: &corev1.ConfigMap{Name: "ca", Namespace: namespace, Data: map[string]string{"ca.crt": string(configMapCA)}},
			want:      configMapCA,
		},
		{
			name:   "Secret",
			ref:    gwv1.LocalObjectReference{Group: "", Kind: "Secret", Name: "ca"},
			secret: &corev1.Secret{Name: "ca", Namespace: namespace, Data: map[string][]byte{"ca.crt": secretCA}},
			want:   secretCA,
		},
		{
			// Kind is required by the CRD, but an empty one must not be treated as unsupported.
			name:      "omitted kind defaults to ConfigMap",
			ref:       gwv1.LocalObjectReference{Group: "", Name: "ca"},
			configMap: &corev1.ConfigMap{Name: "ca", Namespace: namespace, Data: map[string]string{"ca.crt": string(configMapCA)}},
			want:      configMapCA,
		},
		{
			name:      "Secret ref does not fall back to a same-name ConfigMap",
			ref:       gwv1.LocalObjectReference{Group: "", Kind: "Secret", Name: "ca"},
			configMap: &corev1.ConfigMap{Name: "ca", Namespace: namespace, Data: map[string]string{"ca.crt": string(configMapCA)}},
			wantErr:   "Secret default/ca not found",
			wantIs:    cacert.ErrNotFound,
		},
		{
			name:    "missing ConfigMap",
			ref:     gwv1.LocalObjectReference{Group: "", Kind: "ConfigMap", Name: "ca"},
			wantErr: "ConfigMap default/ca not found",
			wantIs:  cacert.ErrNotFound,
		},
		{
			name:    "Secret missing ca.crt",
			ref:     gwv1.LocalObjectReference{Group: "", Kind: "Secret", Name: "ca"},
			secret:  &corev1.Secret{Name: "ca", Namespace: namespace},
			wantErr: "missing ca.crt",
		},
		{
			name:    "Secret with invalid PEM",
			ref:     gwv1.LocalObjectReference{Group: "", Kind: "Secret", Name: "ca"},
			secret:  &corev1.Secret{Name: "ca", Namespace: namespace, Data: map[string][]byte{"ca.crt": []byte("not pem")}},
			wantErr: "invalid ca.crt in Secret default/ca",
		},
		{
			name:    "unsupported kind",
			ref:     gwv1.LocalObjectReference{Group: "", Kind: "Service", Name: "ca"},
			wantErr: `unsupported CA certificate reference kind "Service"`,
			wantIs:  cacert.ErrUnsupportedKind,
		},
		{
			name:    "non-core group",
			ref:     gwv1.LocalObjectReference{Group: "example.com", Kind: "ConfigMap", Name: "ca"},
			wantErr: "only the core API group is supported",
			wantIs:  cacert.ErrUnsupportedKind,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var configMaps []*corev1.ConfigMap
			if tt.configMap != nil {
				configMaps = []*corev1.ConfigMap{tt.configMap}
			}
			var secrets []*corev1.Secret
			if tt.secret != nil {
				secrets = []*corev1.Secret{tt.secret}
			}
			got, err := cacert.ResolveGatewayRef(
				krt.TestingDummyContext{},
				krt.NewStaticCollection(nil, configMaps, krt.WithName("cacert/ConfigMaps")),
				krt.NewStaticCollection(nil, secrets, krt.WithName("cacert/Secrets")),
				namespace,
				tt.ref,
			)

			if tt.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("error = %v, want substring %q", err, tt.wantErr)
				}
				if tt.wantIs != nil && !errors.Is(err, tt.wantIs) {
					t.Fatalf("error = %v, want errors.Is %v", err, tt.wantIs)
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
