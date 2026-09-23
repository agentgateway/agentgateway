package cacert

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/client-go/util/cert"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

// DefaultKey is the data key a CA bundle is read from when a reference does not name one.
const DefaultKey = corev1.ServiceAccountRootCAKey

// Kind returns the selected Kubernetes source kind for a CA reference.
func Kind(kind string) string {
	if kind == "" {
		return "ConfigMap"
	}
	return kind
}

// Key returns the data key ref reads its CA bundle from, defaulting to ca.crt.
func Key(ref agentgateway.LocalCACertificateRef) string {
	if ref.Key == "" {
		return DefaultKey
	}
	return ref.Key
}

// Resolve validates and normalizes the CA certificate selected by ref. The bundle is read from the
// key named by ref.Key, or ca.crt when it is unset.
func Resolve(
	krtctx krt.HandlerContext,
	configMaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	ref agentgateway.LocalCACertificateRef,
) (string, error) {
	nn := types.NamespacedName{Namespace: namespace, Name: string(ref.Name)}
	kind := Kind(ref.Kind)
	key := Key(ref)
	var caCRT []byte

	switch kind {
	case "ConfigMap":
		configMap := ptr.Flatten(krt.FetchOne(krtctx, configMaps, krt.FilterObjectName(nn)))
		if configMap == nil {
			return "", fmt.Errorf("ConfigMap %s not found", nn)
		}
		value, ok := configMap.Data[key]
		if !ok || value == "" {
			return "", fmt.Errorf("error extracting CA cert from ConfigMap %s: missing key %q", nn, key)
		}
		caCRT = []byte(value)
	case "Secret":
		secret := ptr.Flatten(krt.FetchOne(krtctx, secrets, krt.FilterObjectName(nn)))
		if secret == nil {
			return "", fmt.Errorf("Secret %s not found", nn)
		}
		var ok bool
		caCRT, ok = secret.Data[key]
		if !ok || len(caCRT) == 0 {
			return "", fmt.Errorf("error extracting CA cert from Secret %s: missing key %q", nn, key)
		}
	default:
		return "", fmt.Errorf("unsupported CA certificate reference kind %q", ref.Kind)
	}

	certificates, err := cert.ParseCertsPEM(caCRT)
	if err != nil {
		return "", fmt.Errorf("invalid CA certificate in %s %s key %q: %w", kind, nn, key, err)
	}
	normalized, err := cert.EncodeCertificates(certificates...)
	if err != nil {
		return "", fmt.Errorf("invalid CA certificate in %s %s key %q: %w", kind, nn, key, err)
	}
	return string(normalized), nil
}
