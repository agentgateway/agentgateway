package cacert

import (
	"errors"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/util/cert"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

const (
	// KindConfigMap is the ConfigMap CA source kind, and the default when a reference omits its kind.
	KindConfigMap = "ConfigMap"
	// KindSecret is the Secret CA source kind.
	KindSecret = "Secret"
)

var (
	// ErrUnsupportedKind reports a CA reference naming something other than a core ConfigMap or Secret.
	ErrUnsupportedKind = errors.New("unsupported CA certificate reference kind")
	// ErrNotFound reports a CA reference whose target object does not exist.
	ErrNotFound = errors.New("not found")
)

// Kind returns the selected Kubernetes source kind for a CA reference.
func Kind(kind string) string {
	if kind == "" {
		return KindConfigMap
	}
	return kind
}

// Resolve validates and normalizes the CA certificate selected by ref.
func Resolve(
	krtctx krt.HandlerContext,
	configMaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	ref agentgateway.LocalCACertificateRef,
) (string, error) {
	return ResolveSource(krtctx, configMaps, secrets, namespace, Kind(ref.Kind), string(ref.Name))
}

// GatewayRefKind returns the source kind selected by an upstream Gateway API reference, as used by
// BackendTLSPolicy and XBackend. A ConfigMap has "Core" support upstream; a Secret is an
// implementation-specific extension that upstream explicitly permits. Anything else, including any
// non-core group, is ErrUnsupportedKind.
func GatewayRefKind(ref gwv1.LocalObjectReference) (string, error) {
	if ref.Group != "" {
		return "", fmt.Errorf("%w %q: only the core API group is supported", ErrUnsupportedKind, string(ref.Group)+"/"+string(ref.Kind))
	}
	kind := Kind(string(ref.Kind))
	if kind != KindConfigMap && kind != KindSecret {
		return "", fmt.Errorf("%w %q", ErrUnsupportedKind, kind)
	}
	return kind, nil
}

// ResolveGatewayRef validates and normalizes the CA certificate selected by an upstream Gateway API
// reference.
func ResolveGatewayRef(
	krtctx krt.HandlerContext,
	configMaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	ref gwv1.LocalObjectReference,
) (string, error) {
	kind, err := GatewayRefKind(ref)
	if err != nil {
		return "", err
	}
	return ResolveSource(krtctx, configMaps, secrets, namespace, kind, string(ref.Name))
}

// ResolveSource validates and normalizes the CA certificate held under the ca.crt key of the named
// ConfigMap or Secret.
func ResolveSource(
	krtctx krt.HandlerContext,
	configMaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	kind string,
	name string,
) (string, error) {
	nn := types.NamespacedName{Namespace: namespace, Name: name}
	var caCRT []byte

	switch kind {
	case KindConfigMap:
		configMap := ptr.Flatten(krt.FetchOne(krtctx, configMaps, krt.FilterObjectName(nn)))
		if configMap == nil {
			return "", fmt.Errorf("ConfigMap %s %w", nn, ErrNotFound)
		}
		value, ok := configMap.Data[corev1.ServiceAccountRootCAKey]
		if !ok || value == "" {
			return "", fmt.Errorf("error extracting CA cert from ConfigMap %s: missing ca.crt", nn)
		}
		caCRT = []byte(value)
	case KindSecret:
		secret := ptr.Flatten(krt.FetchOne(krtctx, secrets, krt.FilterObjectName(nn)))
		if secret == nil {
			return "", fmt.Errorf("Secret %s %w", nn, ErrNotFound)
		}
		var ok bool
		caCRT, ok = secret.Data[corev1.ServiceAccountRootCAKey]
		if !ok || len(caCRT) == 0 {
			return "", fmt.Errorf("error extracting CA cert from Secret %s: missing ca.crt", nn)
		}
	default:
		return "", fmt.Errorf("%w %q", ErrUnsupportedKind, kind)
	}

	certificates, err := cert.ParseCertsPEM(caCRT)
	if err != nil {
		return "", fmt.Errorf("invalid ca.crt in %s %s: %w", kind, nn, err)
	}
	normalized, err := cert.EncodeCertificates(certificates...)
	if err != nil {
		return "", fmt.Errorf("invalid ca.crt in %s %s: %w", kind, nn, err)
	}
	return string(normalized), nil
}
