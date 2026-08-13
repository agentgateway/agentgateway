package plugins

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
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestTranslateBackendTLSCAReference(t *testing.T) {
	configMapCA := testCAPEM(t, 1)
	secretCA := testCAPEM(t, 2)

	tests := []struct {
		name      string
		ref       agentgateway.LocalCACertificateRef
		configMap *corev1.ConfigMap
		secret    *corev1.Secret
		wantRoot  []byte
		wantErr   string
	}{
		{
			name: "secret wins same-name collision",
			ref:  agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			configMap: &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string]string{
				"ca.crt": string(configMapCA),
			}},
			secret: &corev1.Secret{
				ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"},
				Type:       corev1.SecretTypeTLS,
				Data:       map[string][]byte{"ca.crt": secretCA},
			},
			wantRoot: secretCA,
		},
		{
			name: "explicit configmap",
			ref:  agentgateway.LocalCACertificateRef{Name: "ca", Kind: "ConfigMap"},
			configMap: &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string]string{
				"ca.crt": string(configMapCA),
			}},
			secret: &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string][]byte{
				"ca.crt": secretCA,
			}},
			wantRoot: configMapCA,
		},
		{
			name: "omitted kind defaults to configmap",
			ref:  agentgateway.LocalCACertificateRef{Name: "ca"},
			configMap: &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string]string{
				"ca.crt": string(configMapCA),
			}},
			secret: &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string][]byte{
				"ca.crt": secretCA,
			}},
			wantRoot: configMapCA,
		},
		{
			name: "missing selected secret does not fall back",
			ref:  agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			configMap: &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string]string{
				"ca.crt": string(configMapCA),
			}},
			wantErr: "Secret default/ca not found",
		},
		{
			name: "missing selected configmap does not fall back",
			ref:  agentgateway.LocalCACertificateRef{Name: "ca", Kind: "ConfigMap"},
			secret: &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string][]byte{
				"ca.crt": secretCA,
			}},
			wantErr: "ConfigMap default/ca not found",
		},
		{
			name:    "secret missing ca key",
			ref:     agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			secret:  &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}},
			wantErr: "missing ca.crt",
		},
		{
			name: "secret has invalid pem",
			ref:  agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			secret: &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string][]byte{
				"ca.crt": []byte("not pem"),
			}},
			wantErr: "invalid ca.crt in Secret default/ca",
		},
		{
			name: "configmap has invalid pem",
			ref:  agentgateway.LocalCACertificateRef{Name: "ca", Kind: "ConfigMap"},
			configMap: &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: "default"}, Data: map[string]string{
				"ca.crt": "not pem",
			}},
			wantErr: "invalid ca.crt in ConfigMap default/ca",
		},
		{
			name:    "unsupported kind",
			ref:     agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Service"},
			wantErr: "unsupported CA certificate reference kind",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ctx := backendTLSContext(t, tt.configMap, tt.secret)
			policies, err := TranslateInlineBackendPolicy(ctx, "default", &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					TLS: &agentgateway.BackendTLS{CACertificateRefs: []agentgateway.LocalCACertificateRef{tt.ref}},
				},
			})

			if tt.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), tt.wantErr) {
					t.Fatalf("error = %v, want substring %q", err, tt.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if len(policies) != 1 || policies[0].GetBackendTls() == nil {
				t.Fatalf("translated policies = %#v, want one backend TLS policy", policies)
			}
			if got := policies[0].GetBackendTls().GetRoot(); string(got) != string(tt.wantRoot) {
				t.Fatalf("root CA = %q, want %q", got, tt.wantRoot)
			}
		})
	}
}

func TestTranslateBackendTLSCAErrorUpdatesPolicyStatus(t *testing.T) {
	collections := backendTLSContext(t, nil, nil).Collections
	collections.Gateways = krt.NewStaticCollection[*gwv1.Gateway](nil, []*gwv1.Gateway{{
		ObjectMeta: metav1.ObjectMeta{Name: "gateway", Namespace: "default"},
	}}, krt.WithName("plugins/backendTLSStatusGateways"))
	policy := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{Name: "policy", Namespace: "default", Generation: 1},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []agentgateway.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: agentgateway.LocalPolicyTargetReference{
					Group: wellknown.GatewayGroup,
					Kind:  wellknown.GatewayKind,
					Name:  "gateway",
				},
			}},
			Backend: &agentgateway.BackendFull{BackendSimple: agentgateway.BackendSimple{
				TLS: &agentgateway.BackendTLS{CACertificateRefs: []agentgateway.LocalCACertificateRef{{Name: "missing", Kind: "Secret"}}},
			}},
		},
	}

	references := BuildReferenceIndex(nil, nil, DefaultReferenceTypes(collections))
	status, translated := TranslateAgentgatewayPolicy(krt.TestingDummyContext{}, policy, collections, references, nil, nil, nil, nil)
	if len(translated) != 1 {
		t.Fatalf("translated policies = %d, want 1", len(translated))
	}
	if len(status.Ancestors) != 1 || len(status.Ancestors[0].Conditions) == 0 {
		t.Fatalf("status ancestors = %#v, want one ancestor with conditions", status.Ancestors)
	}
	condition := status.Ancestors[0].Conditions[0]
	if condition.Type != string(agentgateway.PolicyConditionAccepted) || condition.Status != metav1.ConditionTrue || condition.Reason != string(agentgateway.PolicyReasonPartiallyValid) {
		t.Fatalf("accepted condition = %#v, want partially valid", condition)
	}
	if !strings.Contains(condition.Message, "Secret default/missing not found") {
		t.Fatalf("accepted condition message = %q, want missing Secret error", condition.Message)
	}
}

func backendTLSContext(t *testing.T, configMap *corev1.ConfigMap, secret *corev1.Secret) PolicyCtx {
	t.Helper()
	var configMaps []*corev1.ConfigMap
	if configMap != nil {
		configMaps = []*corev1.ConfigMap{configMap}
	}
	var secrets []*corev1.Secret
	if secret != nil {
		secrets = []*corev1.Secret{secret}
	}
	collections := &AgwCollections{
		ConfigMaps:     krt.NewStaticCollection[*corev1.ConfigMap](nil, configMaps, krt.WithName("plugins/backendTLSConfigMaps")),
		Secrets:        krt.NewStaticCollection[*corev1.Secret](nil, secrets, krt.WithName("plugins/backendTLSSecrets")),
		ControllerName: "agentgateway.dev/controller",
	}
	return PolicyCtx{Krt: krt.TestingDummyContext{}, Collections: collections}
}

func testCAPEM(t *testing.T, serial int64) []byte {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	certificate, err := x509.CreateCertificate(rand.Reader, &x509.Certificate{
		SerialNumber:          big.NewInt(serial),
		NotBefore:             now,
		NotAfter:              now.Add(time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}, &x509.Certificate{
		SerialNumber:          big.NewInt(serial),
		NotBefore:             now,
		NotAfter:              now.Add(time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certificate})
}
