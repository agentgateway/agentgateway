package remotehttp_test

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/x509"
	"encoding/pem"
	"math/big"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/policyselection"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
)

func TestResolveSecretCAReactsToRotation(t *testing.T) {
	const namespace = "default"
	stop := test.NewStop(t)
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace},
		Data:       map[string][]byte{"ca.crt": testCAPEM(t)},
	}
	services := krt.NewStaticCollection(nil, []*corev1.Service{
		testService([]corev1.ServicePort{{Name: "https", Port: 8443}}),
	}, krt.WithStop(stop))
	secrets := krt.NewStaticCollection(nil, []*corev1.Secret{secret}, krt.WithStop(stop))
	configMaps := krt.NewStaticCollection[*corev1.ConfigMap](nil, nil, krt.WithStop(stop))
	policies := krt.NewStaticCollection(nil, []*agentgateway.AgentgatewayPolicy{{
		ObjectMeta: metav1.ObjectMeta{Name: "backend-policy", Namespace: namespace},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: []agentgateway.LocalPolicyTargetReferenceWithSectionName{{
				LocalPolicyTargetReference: agentgateway.LocalPolicyTargetReference{Kind: "Service", Name: "oauth2-discovery"},
			}},
			Backend: &agentgateway.BackendFull{BackendSimple: agentgateway.BackendSimple{
				TLS: &agentgateway.BackendTLS{CACertificateRefs: []agentgateway.LocalCACertificateRef{{Name: "ca", Kind: "Secret"}}},
			}},
		},
	}}, krt.WithStop(stop))
	backendTLSPolicies := krt.NewStaticCollection[*gwv1.BackendTLSPolicy](nil, nil, krt.WithStop(stop))
	resolver := remotehttp.NewResolver(remotehttp.Inputs{
		ConfigMaps:     configMaps,
		Secrets:        secrets,
		Services:       services,
		PolicySelector: policyselection.NewSelector(policies, backendTLSPolicies),
	})
	resolved := krt.NewCollection(policies, func(ctx krt.HandlerContext, policy *agentgateway.AgentgatewayPolicy) *resolvedCABundleHash {
		target, err := resolver.Resolve(ctx, remotehttp.ResolveInput{
			ParentName:       policy.Name,
			DefaultNamespace: namespace,
			BackendRef:       &gwv1.BackendObjectReference{Name: "oauth2-discovery", Kind: kindPtr("Service"), Port: port(8443)},
			Path:             "/",
		})
		if err != nil {
			return &resolvedCABundleHash{Key: policy.Name, Err: err.Error()}
		}
		return &resolvedCABundleHash{Key: policy.Name, Hash: target.Target.Transport.CABundleHash}
	}, krt.WithStop(stop))

	hashes := make(chan resolvedCABundleHash, 2)
	registration := resolved.Register(func(event krt.Event[resolvedCABundleHash]) {
		hashes <- event.Latest()
	})
	t.Cleanup(registration.UnregisterHandler)

	initial := <-hashes
	require.Empty(t, initial.Err)
	require.NotEmpty(t, initial.Hash)

	rotated := secret.DeepCopy()
	rotated.Data["ca.crt"] = testCAPEM(t)
	secrets.UpdateObject(rotated)

	updated := <-hashes
	require.Empty(t, updated.Err)
	require.NotEqual(t, initial.Hash, updated.Hash)
}

type resolvedCABundleHash struct {
	Key  string
	Hash string
	Err  string
}

func (h resolvedCABundleHash) ResourceName() string {
	return h.Key
}

func TestResolveSecretCA(t *testing.T) {
	const namespace = "default"
	ctx := testutils.BuildMockPolicyContext(t, []any{
		testService([]corev1.ServicePort{{Name: "https", Port: 8443}}),
		// A same-name ConfigMap must not be used for an explicit Secret ref.
		&corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace}},
		&corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace},
			Data:       map[string][]byte{"ca.crt": testCAPEM(t)},
		},
		&agentgateway.AgentgatewayPolicy{
			ObjectMeta: metav1.ObjectMeta{Name: "backend-policy", Namespace: namespace},
			Spec: agentgateway.AgentgatewayPolicySpec{
				TargetRefs: []agentgateway.LocalPolicyTargetReferenceWithSectionName{{
					LocalPolicyTargetReference: agentgateway.LocalPolicyTargetReference{
						Group: "", Kind: "Service", Name: "oauth2-discovery",
					},
				}},
				Backend: &agentgateway.BackendFull{BackendSimple: agentgateway.BackendSimple{
					TLS: &agentgateway.BackendTLS{CACertificateRefs: []agentgateway.LocalCACertificateRef{{Name: "ca", Kind: "Secret"}}},
				}},
			},
		},
	})

	resolved, err := testutils.BuildRemoteHTTPResolver(ctx.Collections).Resolve(ctx.Krt, remotehttp.ResolveInput{
		ParentName:       "test",
		DefaultNamespace: namespace,
		BackendRef:       &gwv1.BackendObjectReference{Name: "oauth2-discovery", Kind: kindPtr("Service"), Port: port(8443)},
		Path:             "/",
	})
	require.NoError(t, err)
	require.NotEmpty(t, resolved.Target.Transport.CABundleHash)
}

func TestResolveCAReferenceKindsAndErrors(t *testing.T) {
	const namespace = "default"
	validCA := testCAPEM(t)
	tests := []struct {
		name    string
		ref     agentgateway.LocalCACertificateRef
		source  any
		wantErr string
	}{
		{
			name:   "name-only ConfigMap",
			ref:    agentgateway.LocalCACertificateRef{Name: "ca"},
			source: &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace}, Data: map[string]string{"ca.crt": string(validCA)}},
		},
		{
			name:   "explicit ConfigMap",
			ref:    agentgateway.LocalCACertificateRef{Name: "ca", Kind: "ConfigMap"},
			source: &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace}, Data: map[string]string{"ca.crt": string(validCA)}},
		},
		{
			name:    "missing Secret",
			ref:     agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			wantErr: "Secret default/ca not found",
		},
		{
			name:    "Secret missing ca.crt",
			ref:     agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			source:  &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace}},
			wantErr: "missing ca.crt",
		},
		{
			name:    "Secret with invalid PEM",
			ref:     agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			source:  &corev1.Secret{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace}, Data: map[string][]byte{"ca.crt": []byte("invalid")}},
			wantErr: "invalid ca.crt in Secret default/ca",
		},
		{
			name:    "Secret does not fall back to ConfigMap",
			ref:     agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Secret"},
			source:  &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "ca", Namespace: namespace}, Data: map[string]string{"ca.crt": string(validCA)}},
			wantErr: "Secret default/ca not found",
		},
		{
			name:    "unsupported kind",
			ref:     agentgateway.LocalCACertificateRef{Name: "ca", Kind: "Other"},
			wantErr: "unsupported CA certificate reference kind \"Other\"",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			inputs := []any{
				testService([]corev1.ServicePort{{Name: "https", Port: 8443}}),
				&agentgateway.AgentgatewayPolicy{
					ObjectMeta: metav1.ObjectMeta{Name: "backend-policy", Namespace: namespace},
					Spec: agentgateway.AgentgatewayPolicySpec{
						TargetRefs: []agentgateway.LocalPolicyTargetReferenceWithSectionName{{
							LocalPolicyTargetReference: agentgateway.LocalPolicyTargetReference{Kind: "Service", Name: "oauth2-discovery"},
						}},
						Backend: &agentgateway.BackendFull{BackendSimple: agentgateway.BackendSimple{
							TLS: &agentgateway.BackendTLS{CACertificateRefs: []agentgateway.LocalCACertificateRef{tt.ref}},
						}},
					},
				},
			}
			if tt.source != nil {
				inputs = append(inputs, tt.source)
			}
			ctx := testutils.BuildMockPolicyContext(t, inputs)
			resolved, err := testutils.BuildRemoteHTTPResolver(ctx.Collections).Resolve(ctx.Krt, remotehttp.ResolveInput{
				ParentName:       "test",
				DefaultNamespace: namespace,
				BackendRef:       &gwv1.BackendObjectReference{Name: "oauth2-discovery", Kind: kindPtr("Service"), Port: port(8443)},
				Path:             "/",
			})
			if tt.wantErr != "" {
				require.ErrorContains(t, err, tt.wantErr)
				return
			}
			require.NoError(t, err)
			require.NotEmpty(t, resolved.Target.Transport.CABundleHash)
		})
	}
}

func testCAPEM(t *testing.T) []byte {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	require.NoError(t, err)
	template := &x509.Certificate{
		SerialNumber:          big.NewInt(1),
		NotBefore:             time.Now(),
		NotAfter:              time.Now().Add(time.Hour),
		IsCA:                  true,
		BasicConstraintsValid: true,
		KeyUsage:              x509.KeyUsageCertSign,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	require.NoError(t, err)
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
}

func kindPtr(value string) *gwv1.Kind {
	kind := gwv1.Kind(value)
	return &kind
}

func port(value int32) *gwv1.PortNumber {
	port := gwv1.PortNumber(value)
	return &port
}
