package kubeutils

import (
	"errors"
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestResolveCredentialRef(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "creds",
		},
		Data: map[string][]byte{
			"token": []byte("value"),
		},
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("kubeutils/TestResolveCredentialRef"))
	secretCredResolver := NewSecretCredentialResolver(secrets)

	tests := []struct {
		name            string
		ref             agentgateway.LocalSecretObjectRef
		want            map[string][]byte
		wantErrContains string
		wantErrType     error
	}{
		{
			name: "default secret",
			ref:  agentgateway.LocalSecretObjectRef{Name: "creds"},
			want: secret.Data,
		},
		{
			name: "explicit core secret",
			ref: agentgateway.LocalSecretObjectRef{
				Name:  "creds",
				Group: "",
				Kind:  "Secret",
			},
			want: secret.Data,
		},
		{
			name:            "missing secret",
			ref:             agentgateway.LocalSecretObjectRef{Name: "missing"},
			wantErrContains: "secret default/missing not found",
		},
		{
			name:            "empty name",
			ref:             agentgateway.LocalSecretObjectRef{},
			wantErrContains: "credential ref name is required",
		},
		{
			name: "unsupported kind",
			ref: agentgateway.LocalSecretObjectRef{
				Name:  "creds",
				Group: "agentgateway.dev",
				Kind:  "FileCredential",
			},
			wantErrContains: "unsupported credential kind",
			wantErrType:     ErrUnsupportedCredentialKind,
		},
		{
			name: "unsupported kind without name",
			ref: agentgateway.LocalSecretObjectRef{
				Group: "agentgateway.dev",
				Kind:  "FileCredential",
			},
			wantErrContains: "unsupported credential kind",
			wantErrType:     ErrUnsupportedCredentialKind,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := secretCredResolver.ResolveCredentialRef(krt.TestingDummyContext{}, tt.ref, "default")
			if tt.wantErrContains != "" {
				if err == nil {
					t.Fatal("ResolveCredentialRef() error = nil, want error")
				}
				if tt.wantErrType != nil && !errors.Is(err, tt.wantErrType) {
					t.Fatalf("ResolveCredentialRef() error = %v, want ErrUnsupportedCredentialKind", err)
				}
				if !strings.Contains(err.Error(), tt.wantErrContains) {
					t.Fatalf("ResolveCredentialRef() error = %q, want substring %q", err.Error(), tt.wantErrContains)
				}
				return
			}
			if err != nil {
				t.Fatalf("ResolveCredentialRef() error = %v", err)
			}
			if string(got["token"]) != string(tt.want["token"]) {
				t.Fatalf("ResolveCredentialRef() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestChainedCredentialResolver(t *testing.T) {
	resolver := NewChainedCredentialResolver(
		staticCredentialResolver{
			group: "example.agentgateway.dev",
			kind:  "FileCredential",
			data:  map[string][]byte{"token": []byte("custom")},
		},
		NewChainedCredentialResolver(
			nil,
			staticCredentialResolver{
				group: "",
				kind:  "Secret",
				data:  map[string][]byte{"token": []byte("secret")},
			},
		),
	)

	tests := []struct {
		name      string
		ref       agentgateway.LocalSecretObjectRef
		wantToken string
		wantErr   error
	}{
		{
			name: "custom group kind",
			ref: agentgateway.LocalSecretObjectRef{
				Name:  "custom-creds",
				Group: "example.agentgateway.dev",
				Kind:  "FileCredential",
			},
			wantToken: "custom",
		},
		{
			name:      "name only defaults to secret",
			ref:       agentgateway.LocalSecretObjectRef{Name: "secret-creds"},
			wantToken: "secret",
		},
		{
			name: "explicit core secret",
			ref: agentgateway.LocalSecretObjectRef{
				Name: "secret-creds",
				Kind: "Secret",
			},
			wantToken: "secret",
		},
		{
			name: "unsupported",
			ref: agentgateway.LocalSecretObjectRef{
				Name:  "custom-creds",
				Group: "unknown.agentgateway.dev",
				Kind:  "FileCredential",
			},
			wantErr: ErrUnsupportedCredentialKind,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := resolver.ResolveCredentialRef(krt.TestingDummyContext{}, tt.ref, "default")
			if tt.wantErr != nil {
				if !errors.Is(err, tt.wantErr) {
					t.Fatalf("ResolveCredentialRef() error = %v, want %v", err, tt.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("ResolveCredentialRef() error = %v", err)
			}
			if string(got["token"]) != tt.wantToken {
				t.Fatalf("ResolveCredentialRef()[token] = %q, want %q", got["token"], tt.wantToken)
			}
		})
	}
}

type staticCredentialResolver struct {
	group string
	kind  string
	data  map[string][]byte
}

func (r staticCredentialResolver) ResolveCredentialRef(_ krt.HandlerContext, ref agentgateway.LocalSecretObjectRef, _ string) (map[string][]byte, error) {
	if ref.Group != r.group {
		return nil, ErrUnsupportedCredentialKind
	}
	if r.group == "" && ref.Kind == "" && r.kind == "Secret" {
		return r.data, nil
	}
	if ref.Kind != r.kind {
		return nil, ErrUnsupportedCredentialKind
	}
	return r.data, nil
}

func TestGetSecretDataValue(t *testing.T) {
	tests := []struct {
		name      string
		data      map[string][]byte
		key       string
		wantValue string
		wantFound bool
	}{
		{
			name:      "valid secret value",
			data:      map[string][]byte{"key1": []byte("value1")},
			key:       "key1",
			wantValue: "value1",
			wantFound: true,
		},
		{
			name:      "secret value with spaces",
			data:      map[string][]byte{"key1": []byte("  value with spaces  ")},
			key:       "key1",
			wantValue: "value with spaces",
			wantFound: true,
		},
		{
			name:      "key not found",
			data:      map[string][]byte{"other-key": []byte("value")},
			key:       "missing-key",
			wantFound: false,
		},
		{
			name:      "invalid UTF-8",
			data:      map[string][]byte{"key1": {0xff, 0xfe, 0xfd}},
			key:       "key1",
			wantFound: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotValue, gotFound := GetSecretDataValue(tt.data, tt.key)
			if gotFound != tt.wantFound {
				t.Fatalf("GetSecretDataValue() found = %v, want %v", gotFound, tt.wantFound)
			}
			if gotValue != tt.wantValue {
				t.Fatalf("GetSecretDataValue() value = %q, want %q", gotValue, tt.wantValue)
			}
		})
	}
}

func TestGetSecretDataAuth(t *testing.T) {
	tests := []struct {
		name      string
		data      map[string][]byte
		wantValue string
		wantFound bool
	}{
		{
			name:      "strips bearer prefix",
			data:      map[string][]byte{"Authorization": []byte("Bearer token")},
			wantValue: "token",
			wantFound: true,
		},
		{
			name:      "plain value",
			data:      map[string][]byte{"Authorization": []byte("token")},
			wantValue: "token",
			wantFound: true,
		},
		{
			name:      "bare bearer value",
			data:      map[string][]byte{"Authorization": []byte("Bearer   ")},
			wantValue: "Bearer",
			wantFound: true,
		},
		{
			name:      "missing",
			data:      map[string][]byte{},
			wantFound: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotValue, gotFound := GetSecretDataAuth(tt.data)
			if gotFound != tt.wantFound {
				t.Fatalf("GetSecretDataAuth() found = %v, want %v", gotFound, tt.wantFound)
			}
			if gotValue != tt.wantValue {
				t.Fatalf("GetSecretDataAuth() value = %q, want %q", gotValue, tt.wantValue)
			}
		})
	}
}
