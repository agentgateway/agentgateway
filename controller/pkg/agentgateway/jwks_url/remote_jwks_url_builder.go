package jwks_url

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	krtpkg "github.com/kgateway-dev/kgateway/v2/pkg/utils/krtutil"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

type JwksUrlBuilder interface {
	BuildJwksUrlAndTlsConfig(krtctx krt.HandlerContext, policyName, defaultNS string, remoteProvider *agentgateway.RemoteJWKS) (string, *tls.Config, error)
}

var JwksUrlBuilderFactory = func() JwksUrlBuilder { return &emptyJwksUrlFactory{} }

type emptyJwksUrlFactory struct{}

func (f *emptyJwksUrlFactory) BuildJwksUrlAndTlsConfig(_ krt.HandlerContext, _, _ string, _ *agentgateway.RemoteJWKS) (string, *tls.Config, error) {
	return "", nil, fmt.Errorf("JwksUrlBuilderFactory must be initialized before use")
}

type TargetRefIndexKey struct {
	Group     string
	Kind      string
	Name      string
	Namespace string
}

func (k TargetRefIndexKey) String() string {
	return fmt.Sprintf("%s:%s:%s:%s", k.Group, k.Kind, k.Namespace, k.Name)
}

type defaultJwksUrlFactory struct {
	cfgmaps                  krt.Collection[*corev1.ConfigMap]
	policiesByTargetRefIndex krt.Index[TargetRefIndexKey, *agentgateway.AgentgatewayPolicy]
	backends                 krt.Collection[*agentgateway.AgentgatewayBackend]
	agentgatewayPolicies     krt.Collection[*agentgateway.AgentgatewayPolicy]
}

func NewJwksUrlFactory(cfgmaps krt.Collection[*corev1.ConfigMap],
	backends krt.Collection[*agentgateway.AgentgatewayBackend],
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]) JwksUrlBuilder {
	policiesByTargetRefIndex := krtpkg.UnnamedIndex(agentgatewayPolicies, func(in *agentgateway.AgentgatewayPolicy) []TargetRefIndexKey {
		keys := make([]TargetRefIndexKey, 0)
		for _, ref := range in.Spec.TargetRefs {
			keys = append(keys, TargetRefIndexKey{
				Name:      string(ref.Name),
				Kind:      string(ref.Kind),
				Group:     string(ref.Group),
				Namespace: in.Namespace,
			})
		}
		return keys
	})

	return &defaultJwksUrlFactory{
		cfgmaps:                  cfgmaps,
		policiesByTargetRefIndex: policiesByTargetRefIndex,
		backends:                 backends,
		agentgatewayPolicies:     agentgatewayPolicies,
	}
}

func (f *defaultJwksUrlFactory) BuildJwksUrlAndTlsConfig(krtctx krt.HandlerContext, policyName, defaultNS string, remoteProvider *agentgateway.RemoteJWKS) (string, *tls.Config, error) {
	ref := remoteProvider.BackendRef

	refName := string(ref.Name)
	refNamespace := string(ptr.OrDefault(ref.Namespace, gwv1.Namespace(defaultNS)))

	path := strings.TrimPrefix(remoteProvider.JwksPath, "/")

	switch string(*ref.Kind) {
	case wellknown.AgentgatewayBackendGVK.Kind:
		backendRef := types.NamespacedName{
			Name:      refName,
			Namespace: refNamespace,
		}
		backend := ptr.Flatten(krt.FetchOne(krtctx, f.backends, krt.FilterObjectName(backendRef)))
		if backend == nil {
			return "", nil, fmt.Errorf("backend %s not found, policy %s", backendRef, types.NamespacedName{Namespace: defaultNS, Name: policyName})
		}
		if backend.Spec.Static == nil {
			return "", nil, fmt.Errorf("only static backends are supported; backend: %s, policy: %s", backendRef, types.NamespacedName{Namespace: defaultNS, Name: policyName})
		}

		var tlsConfig *tls.Config
		if backend.Spec.Policies != nil && backend.Spec.Policies.TLS != nil {
			tlsc, err := GetTLSConfig(krtctx, f.cfgmaps, refNamespace, backend.Spec.Policies.TLS)
			if err != nil {
				return "", nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w",
					backendRef, types.NamespacedName{Namespace: refNamespace, Name: policyName}, err)
			}
			tlsConfig = tlsc
		} else {
			agwPolicy := ptr.Flatten(krt.FetchOne(krtctx, f.agentgatewayPolicies, krt.FilterIndex(f.policiesByTargetRefIndex, TargetRefIndexKey{
				Name:      refName,
				Kind:      string(*ref.Kind),
				Group:     string(ptr.OrEmpty(ref.Group)),
				Namespace: refNamespace,
				// no port, as policy targetRef may not have it
				// TODO (dmitri-d) sectionName is optional and we don't know apriori if it's present in the policy's targetRef;
				// so we either ignore it completely (current implementation, an issue if there are multiple policies targeting the same service but different ports),
				// or do multiple searches (with the port set first, then without the port).
			})))

			if agwPolicy != nil && agwPolicy.Spec.Backend != nil && agwPolicy.Spec.Backend.TLS != nil {
				tlsc, err := GetTLSConfig(krtctx, f.cfgmaps, refNamespace, agwPolicy.Spec.Backend.TLS)
				if err != nil {
					return "", nil, fmt.Errorf("error setting tls options; service %s/%s, policy: %s %w",
						refName, refNamespace, types.NamespacedName{Namespace: refNamespace, Name: policyName}, err)
				}
				tlsConfig = tlsc
			}
		}

		var url string
		if tlsConfig == nil {
			url = fmt.Sprintf("http://%s:%d/%s", backend.Spec.Static.Host, backend.Spec.Static.Port, path)
		} else {
			url = fmt.Sprintf("https://%s:%d/%s", backend.Spec.Static.Host, backend.Spec.Static.Port, path)
		}

		return url, tlsConfig, nil
	case wellknown.ServiceKind:
		agwPolicy := ptr.Flatten(krt.FetchOne(krtctx, f.agentgatewayPolicies, krt.FilterIndex(f.policiesByTargetRefIndex, TargetRefIndexKey{
			Name:      refName,
			Kind:      string(*ref.Kind),
			Group:     string(ptr.OrEmpty(ref.Group)),
			Namespace: refNamespace,
			// no port, as policy targetRef may not have it
			// TODO (dmitri-d) sectionName is optional and we don't know apriori if it's present in the policy's targetRef;
			// so we either ignore it completely (current implementation, an issue if there are multiple policies targeting the same service but different ports),
			// or do multiple searches (with the port set first, then without the port).
		})))

		var tlsConfig *tls.Config
		if agwPolicy != nil && agwPolicy.Spec.Backend != nil && agwPolicy.Spec.Backend.TLS != nil {
			tlsc, err := GetTLSConfig(krtctx, f.cfgmaps, refNamespace, agwPolicy.Spec.Backend.TLS)
			if err != nil {
				return "", nil, fmt.Errorf("error setting tls options; service %s/%s, policy: %s %w",
					refName, refNamespace, types.NamespacedName{Namespace: refNamespace, Name: policyName}, err)
			}
			tlsConfig = tlsc
		}

		host := kubeutils.GetServiceHostname(refName, refNamespace)
		var fqdn string
		if port := ptr.OrEmpty(ref.Port); port != 0 {
			fqdn = fmt.Sprintf("%s:%d", host, port)
		} else {
			fqdn = host
		}

		var url string
		if tlsConfig == nil {
			url = fmt.Sprintf("http://%s/%s", fqdn, path)
		} else {
			url = fmt.Sprintf("https://%s/%s", fqdn, path)
		}

		return url, tlsConfig, nil
	}

	return "", nil, fmt.Errorf("unsupported target kind in remote jwks provider; kind: %s, policy: %s", string(*ref.Kind), types.NamespacedName{Namespace: refNamespace, Name: policyName})
}

func GetTLSConfig(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	namespace string,
	btls *agentgateway.BackendTLS,
) (*tls.Config, error) {
	toret := tls.Config{
		ServerName:         ptr.OrEmpty(btls.Sni),
		InsecureSkipVerify: insecureSkipVerify(btls.InsecureSkipVerify), //nolint:gosec
		NextProtos:         ptr.OrEmpty(btls.AlpnProtocols),
	}

	if len(btls.CACertificateRefs) > 0 {
		certPool := x509.NewCertPool()
		for _, ref := range btls.CACertificateRefs {
			nn := types.NamespacedName{
				Name:      string(ref.Name),
				Namespace: namespace,
			}
			cfgmap := krt.FetchOne(krtctx, cfgmaps, krt.FilterObjectName(nn))
			if cfgmap == nil {
				return nil, fmt.Errorf("ConfigMap %s not found", nn)
			}
			success := AppendPoolWithCertsFromConfigMap(certPool, ptr.Flatten(cfgmap))
			if !success {
				return nil, fmt.Errorf("error extracting CA cert from ConfigMap %s", nn)
			}
		}
		toret.RootCAs = certPool
	}

	return &toret, nil
}

func AppendPoolWithCertsFromConfigMap(pool *x509.CertPool, cm *corev1.ConfigMap) bool {
	caCrts, ok := cm.Data["ca.crt"]
	if !ok {
		return false
	}
	return pool.AppendCertsFromPEM([]byte(caCrts))
}

func insecureSkipVerify(mode *agentgateway.InsecureTLSMode) bool {
	return mode != nil
}
