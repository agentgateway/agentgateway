package remotehttp

import (
	"crypto/sha256"
	"crypto/x509"
	"encoding/hex"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/cacert"
)

// caSource identifies one CA certificate source to resolve.
type caSource struct {
	kind string
	name string
}

func caBundleFromBackendTLSRefs(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	refs []agentgateway.LocalCACertificateRef,
) (*x509.CertPool, string, error) {
	sources := make([]caSource, 0, len(refs))
	for _, ref := range refs {
		sources = append(sources, caSource{kind: cacert.Kind(ref.Kind), name: string(ref.Name)})
	}
	return caBundleFromSources(krtctx, cfgmaps, secrets, namespace, sources)
}

func caBundleFromGatewayRefs(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	refs []gwv1.LocalObjectReference,
) (*x509.CertPool, string, error) {
	sources := make([]caSource, 0, len(refs))
	for _, ref := range refs {
		kind, err := cacert.GatewayRefKind(ref)
		if err != nil {
			return nil, "", err
		}
		sources = append(sources, caSource{kind: kind, name: string(ref.Name)})
	}
	return caBundleFromSources(krtctx, cfgmaps, secrets, namespace, sources)
}

func caBundleFromSources(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	secrets krt.Collection[*corev1.Secret],
	namespace string,
	sources []caSource,
) (*x509.CertPool, string, error) {
	certPool := x509.NewCertPool()
	h := sha256.New()

	for _, src := range sources {
		caCRT, err := cacert.ResolveSource(krtctx, cfgmaps, secrets, namespace, src.kind, src.name)
		if err != nil {
			return nil, "", err
		}
		nn := types.NamespacedName{Name: src.name, Namespace: namespace}
		if !certPool.AppendCertsFromPEM([]byte(caCRT)) {
			return nil, "", fmt.Errorf("error appending CA cert from %s %s", src.kind, nn)
		}
		_, _ = h.Write([]byte(src.kind))
		_, _ = h.Write([]byte{0})
		_, _ = h.Write([]byte(nn.String()))
		_, _ = h.Write([]byte{0})
		_, _ = h.Write([]byte(caCRT))
		_, _ = h.Write([]byte{0})
	}

	return certPool, hex.EncodeToString(h.Sum(nil)), nil
}
