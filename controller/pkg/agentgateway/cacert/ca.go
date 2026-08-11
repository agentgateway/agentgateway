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

// Kind returns the selected Kubernetes source kind for a CA reference.
func Kind(kind string) string {
	if kind == "" {
		return "ConfigMap"
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
	nn := types.NamespacedName{Namespace: namespace, Name: string(ref.Name)}
	kind := Kind(ref.Kind)
	var caCRT []byte

	switch kind {
	case "ConfigMap":
		configMap := ptr.Flatten(krt.FetchOne(krtctx, configMaps, krt.FilterObjectName(nn)))
		if configMap == nil {
			return "", fmt.Errorf("ConfigMap %s not found", nn)
		}
		value, ok := configMap.Data[corev1.ServiceAccountRootCAKey]
		if !ok || value == "" {
			return "", fmt.Errorf("error extracting CA cert from ConfigMap %s: missing ca.crt", nn)
		}
		caCRT = []byte(value)
	case "Secret":
		secret := ptr.Flatten(krt.FetchOne(krtctx, secrets, krt.FilterObjectName(nn)))
		if secret == nil {
			return "", fmt.Errorf("Secret %s not found", nn)
		}
		var ok bool
		caCRT, ok = secret.Data[corev1.ServiceAccountRootCAKey]
		if !ok || len(caCRT) == 0 {
			return "", fmt.Errorf("error extracting CA cert from Secret %s: missing ca.crt", nn)
		}
	default:
		return "", fmt.Errorf("unsupported CA certificate reference kind %q", ref.Kind)
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
