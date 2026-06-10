package kubeutils

import (
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

// ErrUnsupportedCredentialKind signals that a resolver does not own the
// CredentialRef group/kind, allowing a chained resolver to try the next
// resolver.
var ErrUnsupportedCredentialKind = errors.New("unsupported credential kind")

// CredentialResolver resolves same-namespace credential refs into data keyed by
// credential field name. Implementations should return
// ErrUnsupportedCredentialKind for group/kind pairs they do not handle; missing
// data for a handled ref should return a normal error.
type CredentialResolver interface {
	ResolveCredentialRef(krtctx krt.HandlerContext, ref agentgateway.LocalSecretObjectRef, namespace string) (map[string][]byte, error)
}

// NewChainedCredentialResolver returns a resolver that tries resolvers in order,
// falling through to the next whenever one returns ErrUnsupportedCredentialKind.
// Nil resolvers are ignored; an empty or all-nil list yields nil.
func NewChainedCredentialResolver(resolvers ...CredentialResolver) CredentialResolver {
	chain := make(chainedCredentialResolver, 0, len(resolvers))
	for _, r := range resolvers {
		if r != nil {
			chain = append(chain, r)
		}
	}
	if len(chain) == 0 {
		return nil
	}
	return chain
}

type chainedCredentialResolver []CredentialResolver

func (r chainedCredentialResolver) ResolveCredentialRef(krtctx krt.HandlerContext, ref agentgateway.LocalSecretObjectRef, namespace string) (map[string][]byte, error) {
	for _, resolver := range r {
		if resolver == nil {
			continue
		}
		data, err := resolver.ResolveCredentialRef(krtctx, ref, namespace)
		if errors.Is(err, ErrUnsupportedCredentialKind) {
			continue
		}
		return data, err
	}
	return nil, fmt.Errorf("%w: %q/%q", ErrUnsupportedCredentialKind, ref.Group, ref.Kind)
}

// NewSecretCredentialResolver returns the built-in resolver for core Secret
// refs: empty group with empty or Secret kind.
func NewSecretCredentialResolver(secrets krt.Collection[*corev1.Secret]) CredentialResolver {
	return secretCredentialResolver{secrets: secrets}
}

type secretCredentialResolver struct {
	secrets krt.Collection[*corev1.Secret]
}

// GetSecret fetches a Kubernetes secret by name and namespace using krt collection.
func GetSecret(secrets krt.Collection[*corev1.Secret], krtctx krt.HandlerContext, secretName, namespace string) (*corev1.Secret, error) {
	secretKey := namespace + "/" + secretName
	secret := ptr.Flatten(krt.FetchOne(krtctx, secrets, krt.FilterKey(secretKey)))
	if secret == nil {
		return nil, fmt.Errorf("secret %s not found", secretKey)
	}
	return secret, nil
}

// ResolveCredentialRef fetches Secret-backed credential bytes for a CredentialRef.
func (r secretCredentialResolver) ResolveCredentialRef(krtctx krt.HandlerContext, ref agentgateway.LocalSecretObjectRef, namespace string) (map[string][]byte, error) {
	if ref.Group != "" || (ref.Kind != "" && ref.Kind != "Secret") {
		return nil, fmt.Errorf("%w: %q/%q", ErrUnsupportedCredentialKind, ref.Group, ref.Kind)
	}
	if ref.Name == "" {
		return nil, errors.New("credential ref name is required")
	}
	if r.secrets == nil {
		return nil, errors.New("credential secret collection is not configured")
	}
	secret, err := GetSecret(r.secrets, krtctx, string(ref.Name), namespace)
	if err != nil {
		return nil, err
	}
	return secret.Data, nil
}

// GetSecretValue extracts a UTF-8 string value from a Kubernetes Secret.
func GetSecretValue(secret *corev1.Secret, key string) (string, bool) {
	if secret == nil {
		return "", false
	}
	return GetSecretDataValue(secret.Data, key)
}

// GetSecretDataValue extracts a UTF-8 string value from Secret data.
func GetSecretDataValue(data map[string][]byte, key string) (string, bool) {
	if value, exists := data[key]; exists && utf8.Valid(value) {
		return strings.TrimSpace(string(value)), true
	}

	return "", false
}

// GetSecretDataAuth extracts an authentication value from Secret data.
// It looks for the "Authorization" field and strips the "Bearer " prefix if present.
func GetSecretDataAuth(data map[string][]byte) (string, bool) {
	if authValue, exists := GetSecretDataValue(data, "Authorization"); exists {
		// Strip the "Bearer " prefix if present, as it will be added by the provider
		authValue = strings.TrimSpace(authValue)
		authKey := strings.TrimSpace(strings.TrimPrefix(authValue, "Bearer "))
		return authKey, authKey != ""
	}
	return "", false
}
