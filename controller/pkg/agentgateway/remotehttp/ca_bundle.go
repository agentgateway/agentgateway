package remotehttp

import (
	"crypto/sha256"
	"crypto/x509"
	"encoding/hex"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

func caBundleFromConfigMaps(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	namespace string,
	refs []corev1.LocalObjectReference,
) (*x509.CertPool, string, error) {
	names := make([]string, 0, len(refs))
	for _, ref := range refs {
		names = append(names, ref.Name)
	}
	return caBundleFromConfigMapNames(krtctx, cfgmaps, namespace, names)
}

func caBundleFromGatewayRefs(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	namespace string,
	refs []gwv1.LocalObjectReference,
) (*x509.CertPool, string, error) {
	names := make([]string, 0, len(refs))
	for _, ref := range refs {
		names = append(names, string(ref.Name))
	}
	return caBundleFromConfigMapNames(krtctx, cfgmaps, namespace, names)
}

func caBundleFromConfigMapNames(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	namespace string,
	names []string,
) (*x509.CertPool, string, error) {
	certPool := x509.NewCertPool()
	h := sha256.New()

	for _, name := range names {
		nn := types.NamespacedName{
			Name:      name,
			Namespace: namespace,
		}
		cfgmap := ptr.Flatten(krt.FetchOne(krtctx, cfgmaps, krt.FilterObjectName(nn)))
		if cfgmap == nil {
			return nil, "", fmt.Errorf("ConfigMap %s not found", nn)
		}
		caCRT, ok := caCertsFromConfigMap(cfgmap)
		if !ok {
			return nil, "", fmt.Errorf("error extracting CA cert from ConfigMap %s", nn)
		}
		if !certPool.AppendCertsFromPEM([]byte(caCRT)) {
			return nil, "", fmt.Errorf("error extracting CA cert from ConfigMap %s", nn)
		}
		_, _ = h.Write([]byte(nn.String()))
		_, _ = h.Write([]byte{0})
		_, _ = h.Write([]byte(caCRT))
		_, _ = h.Write([]byte{0})
	}

	return certPool, hex.EncodeToString(h.Sum(nil)), nil
}

func caCertsFromConfigMap(cm *corev1.ConfigMap) (string, bool) {
	caCrts, ok := cm.Data["ca.crt"]
	if !ok {
		return "", false
	}
	return caCrts, true
}
