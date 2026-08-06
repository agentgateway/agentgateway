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

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/cacert"
)

func caBundleFromBackendTLSRefs(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	refs []agentgateway.LocalCACertificateRef,
) (*x509.CertPool, string, error) {
	certPool := x509.NewCertPool()
	h := sha256.New()

	for _, ref := range refs {
		caCRT, err := cacert.Resolve(krtctx, cfgmaps, secrets, namespace, ref)
		if err != nil {
			return nil, "", err
		}
		nn := types.NamespacedName{Name: string(ref.Name), Namespace: namespace}
		if !certPool.AppendCertsFromPEM([]byte(caCRT)) {
			return nil, "", fmt.Errorf("error appending CA cert from %s %s", cacert.Kind(ref.Kind), nn)
		}
		_, _ = h.Write([]byte(cacert.Kind(ref.Kind)))
		_, _ = h.Write([]byte{0})
		_, _ = h.Write([]byte(nn.String()))
		_, _ = h.Write([]byte{0})
		_, _ = h.Write([]byte(caCRT))
		_, _ = h.Write([]byte{0})
	}

	return certPool, hex.EncodeToString(h.Sum(nil)), nil
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
		nn := types.NamespacedName{Name: name, Namespace: namespace}
		cfgmap := ptr.Flatten(krt.FetchOne(krtctx, cfgmaps, krt.FilterObjectName(nn)))
		if cfgmap == nil {
			return nil, "", fmt.Errorf("ConfigMap %s not found", nn)
		}
		caCRT, ok := cfgmap.Data["ca.crt"]
		if !ok {
			return nil, "", fmt.Errorf("error extracting CA cert from ConfigMap %s: missing ca.crt", nn)
		}
		if !certPool.AppendCertsFromPEM([]byte(caCRT)) {
			return nil, "", fmt.Errorf("error appending CA cert from ConfigMap %s", nn)
		}
		_, _ = h.Write([]byte(nn.String()))
		_, _ = h.Write([]byte{0})
		_, _ = h.Write([]byte(caCRT))
		_, _ = h.Write([]byte{0})
	}

	return certPool, hex.EncodeToString(h.Sum(nil)), nil
}
