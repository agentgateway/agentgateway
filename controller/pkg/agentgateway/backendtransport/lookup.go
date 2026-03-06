package backendtransport

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"slices"
	"strconv"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	krtpkg "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

type targetRefIndexKey struct {
	Group     string
	Kind      string
	Name      string
	Namespace string
}

func (k targetRefIndexKey) String() string {
	return fmt.Sprintf("%s:%s:%s:%s", k.Group, k.Kind, k.Namespace, k.Name)
}

type backendTLSTargetRefIndexKey struct {
	Group     string
	Name      string
	Kind      string
	Namespace string
}

func (k backendTLSTargetRefIndexKey) String() string {
	return fmt.Sprintf("%s:%s:%s:%s", k.Group, k.Namespace, k.Kind, k.Name)
}

type BackendTransportLookup struct {
	cfgmaps                  krt.Collection[*corev1.ConfigMap]
	services                 krt.Collection[*corev1.Service]
	backends                 krt.Collection[*agentgateway.AgentgatewayBackend]
	agentgatewayPolicies     krt.Collection[*agentgateway.AgentgatewayPolicy]
	backendTLSPolicies       krt.Collection[*gwv1.BackendTLSPolicy]
	policiesByTargetRefIndex krt.Index[targetRefIndexKey, *agentgateway.AgentgatewayPolicy]
	backendTLSByTargetIndex  krt.Index[backendTLSTargetRefIndexKey, *gwv1.BackendTLSPolicy]
}

type ResolvedBackendTransport struct {
	ConnectHost string
	TLSConfig   *tls.Config
}

type sectionMatchRank uint8

const (
	sectionNoMatch sectionMatchRank = iota
	sectionWholeResourceMatch
	sectionExactMatch
)

type targetSectionMatcher struct {
	exact      []string
	allowWhole bool
}

// NewBackendTransportLookup builds the shared backend resolution helper used by control-plane
// fetchers such as OIDC discovery and remote JWKS resolution.
func NewBackendTransportLookup(
	cfgmaps krt.Collection[*corev1.ConfigMap],
	services krt.Collection[*corev1.Service],
	backends krt.Collection[*agentgateway.AgentgatewayBackend],
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy],
	backendTLSPolicies krt.Collection[*gwv1.BackendTLSPolicy],
) *BackendTransportLookup {
	return &BackendTransportLookup{
		cfgmaps:              cfgmaps,
		services:             services,
		backends:             backends,
		agentgatewayPolicies: agentgatewayPolicies,
		backendTLSPolicies:   backendTLSPolicies,
		policiesByTargetRefIndex: krtpkg.UnnamedIndex(agentgatewayPolicies, func(in *agentgateway.AgentgatewayPolicy) []targetRefIndexKey {
			keys := make([]targetRefIndexKey, 0, len(in.Spec.TargetRefs))
			for _, ref := range in.Spec.TargetRefs {
				keys = append(keys, targetRefIndexKey{
					Name:      string(ref.Name),
					Kind:      string(ref.Kind),
					Group:     string(ref.Group),
					Namespace: in.Namespace,
				})
			}
			return keys
		}),
		backendTLSByTargetIndex: krtpkg.UnnamedIndex(backendTLSPolicies, func(in *gwv1.BackendTLSPolicy) []backendTLSTargetRefIndexKey {
			keys := make([]backendTLSTargetRefIndexKey, 0, len(in.Spec.TargetRefs))
			for _, ref := range in.Spec.TargetRefs {
				keys = append(keys, backendTLSTargetRefIndexKey{
					Group:     string(ref.Group),
					Name:      string(ref.Name),
					Kind:      string(ref.Kind),
					Namespace: in.Namespace,
				})
			}
			return keys
		}),
	}
}

func (l *BackendTransportLookup) Resolve(
	krtctx krt.HandlerContext,
	policyName, defaultNS string,
	backendRef gwv1.BackendObjectReference,
	defaultPort string,
) (*ResolvedBackendTransport, error) {
	kind := ptr.OrDefault(backendRef.Kind, wellknown.ServiceKind)
	group := ptr.OrDefault(backendRef.Group, "")
	refNamespace := string(ptr.OrDefault(backendRef.Namespace, gwv1.Namespace(defaultNS)))

	switch {
	case string(kind) == wellknown.AgentgatewayBackendGVK.Kind && string(group) == wellknown.AgentgatewayBackendGVK.Group:
		backendNN := types.NamespacedName{Name: string(backendRef.Name), Namespace: refNamespace}
		backend := ptr.Flatten(krt.FetchOne(krtctx, l.backends, krt.FilterObjectName(backendNN)))
		if backend == nil {
			return nil, fmt.Errorf("backend %s not found, policy %s", backendNN, types.NamespacedName{Namespace: defaultNS, Name: policyName})
		}
		if backend.Spec.Static == nil {
			return nil, fmt.Errorf("only static backends are supported; backend: %s, policy: %s", backendNN, types.NamespacedName{Namespace: defaultNS, Name: policyName})
		}

		tlsConfig, err := l.resolveTLSConfig(
			krtctx,
			refNamespace,
			string(group),
			string(kind),
			string(backendRef.Name),
			targetSectionMatcher{allowWhole: true},
			backend.Spec.Policies,
		)
		if err != nil {
			return nil, fmt.Errorf("error setting tls options; backend: %s, policy: %s, %w", backendNN, types.NamespacedName{Namespace: defaultNS, Name: policyName}, err)
		}

		return &ResolvedBackendTransport{
			ConnectHost: fmt.Sprintf("%s:%d", backend.Spec.Static.Host, backend.Spec.Static.Port),
			TLSConfig:   tlsConfig,
		}, nil
	case string(kind) == wellknown.ServiceKind && string(group) == "":
		tlsConfig, err := l.resolveTLSConfig(
			krtctx,
			refNamespace,
			string(group),
			string(kind),
			string(backendRef.Name),
			l.serviceTargetSectionMatcher(krtctx, refNamespace, string(backendRef.Name), backendRef.Port, defaultPort),
			nil,
		)
		if err != nil {
			return nil, fmt.Errorf("error setting tls options; service %s/%s, policy: %s, %w", backendRef.Name, refNamespace, types.NamespacedName{Namespace: defaultNS, Name: policyName}, err)
		}

		connectHost := kubeutils.GetServiceHostname(string(backendRef.Name), refNamespace)
		if port := ptr.OrEmpty(backendRef.Port); port != 0 {
			connectHost = fmt.Sprintf("%s:%d", connectHost, port)
		} else if defaultPort != "" {
			connectHost = fmt.Sprintf("%s:%s", connectHost, defaultPort)
		}

		return &ResolvedBackendTransport{
			ConnectHost: connectHost,
			TLSConfig:   tlsConfig,
		}, nil
	default:
		return nil, fmt.Errorf("unsupported backend kind %s.%s for policy %s", group, kind, types.NamespacedName{Namespace: defaultNS, Name: policyName})
	}
}

func (l *BackendTransportLookup) resolveTLSConfig(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	matcher targetSectionMatcher,
	backendPolicies *agentgateway.BackendFull,
) (*tls.Config, error) {
	if backendPolicies != nil && backendPolicies.TLS != nil {
		return GetTLSConfig(krtctx, l.cfgmaps, namespace, backendPolicies.TLS)
	}
	if agwPolicy := l.targetedAgentgatewayPolicy(krtctx, namespace, group, kind, name, matcher); agwPolicy != nil && agwPolicy.Spec.Backend != nil && agwPolicy.Spec.Backend.TLS != nil {
		return GetTLSConfig(krtctx, l.cfgmaps, namespace, agwPolicy.Spec.Backend.TLS)
	}
	if backendTLSPolicy := l.targetedBackendTLSPolicy(krtctx, namespace, group, kind, name, matcher); backendTLSPolicy != nil {
		return tlsConfigFromBackendTLSPolicy(krtctx, l.cfgmaps, namespace, backendTLSPolicy)
	}
	return nil, nil
}

func (l *BackendTransportLookup) targetedAgentgatewayPolicy(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	matcher targetSectionMatcher,
) *agentgateway.AgentgatewayPolicy {
	candidates := krt.Fetch(
		krtctx,
		l.agentgatewayPolicies,
		krt.FilterIndex(l.policiesByTargetRefIndex, targetRefIndexKey{
			Name:      name,
			Kind:      kind,
			Group:     group,
			Namespace: namespace,
		}),
	)
	return bestMatchingAgentgatewayPolicy(candidates, group, kind, name, matcher)
}

func (l *BackendTransportLookup) targetedBackendTLSPolicy(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	matcher targetSectionMatcher,
) *gwv1.BackendTLSPolicy {
	candidates := krt.Fetch(
		krtctx,
		l.backendTLSPolicies,
		krt.FilterIndex(l.backendTLSByTargetIndex, backendTLSTargetRefIndexKey{
			Group:     group,
			Name:      name,
			Kind:      kind,
			Namespace: namespace,
		}),
	)
	return bestMatchingBackendTLSPolicy(candidates, kind, name, matcher)
}

func (l *BackendTransportLookup) serviceTargetSectionMatcher(
	krtctx krt.HandlerContext,
	namespace, name string,
	refPort *gwv1.PortNumber,
	defaultPort string,
) targetSectionMatcher {
	candidates := make([]string, 0, 2)
	appendPort := func(port int32) {
		candidates = append(candidates, strconv.FormatInt(int64(port), 10))
		if portName := l.servicePortName(krtctx, namespace, name, port); portName != "" {
			candidates = append(candidates, portName)
		}
	}

	if port := ptr.OrEmpty(refPort); port != 0 {
		appendPort(int32(port))
	} else if defaultPort != "" {
		if parsed, err := strconv.ParseInt(defaultPort, 10, 32); err == nil {
			appendPort(int32(parsed))
		}
	}

	return targetSectionMatcher{
		exact:      dedupeStrings(candidates),
		allowWhole: true,
	}
}

func (l *BackendTransportLookup) servicePortName(
	krtctx krt.HandlerContext,
	namespace, name string,
	port int32,
) string {
	svc := ptr.Flatten(krt.FetchOne(krtctx, l.services, krt.FilterObjectName(types.NamespacedName{
		Name:      name,
		Namespace: namespace,
	})))
	if svc == nil {
		return ""
	}
	for _, svcPort := range svc.Spec.Ports {
		if svcPort.Port == port {
			return svcPort.Name
		}
	}
	return ""
}

func tlsConfigFromBackendTLSPolicy(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	namespace string,
	policy *gwv1.BackendTLSPolicy,
) (*tls.Config, error) {
	tlsConfig := &tls.Config{
		MinVersion: tls.VersionTLS12,
		ServerName: string(policy.Spec.Validation.Hostname),
	}
	if len(policy.Spec.Validation.CACertificateRefs) == 0 {
		return tlsConfig, nil
	}

	certPool := x509.NewCertPool()
	for _, ref := range policy.Spec.Validation.CACertificateRefs {
		nn := types.NamespacedName{
			Name:      string(ref.Name),
			Namespace: namespace,
		}
		cfgmap := ptr.Flatten(krt.FetchOne(krtctx, cfgmaps, krt.FilterObjectName(nn)))
		if cfgmap == nil {
			return nil, fmt.Errorf("ConfigMap %s not found", nn)
		}
		if !AppendPoolWithCertsFromConfigMap(certPool, cfgmap) {
			return nil, fmt.Errorf("error extracting CA cert from ConfigMap %s", nn)
		}
	}
	tlsConfig.RootCAs = certPool
	return tlsConfig, nil
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
	if caCrts, ok := cm.Data["ca.crt"]; ok {
		return pool.AppendCertsFromPEM([]byte(caCrts))
	}
	return false
}

func insecureSkipVerify(mode *agentgateway.InsecureTLSMode) bool {
	return mode != nil
}

func bestMatchingAgentgatewayPolicy(
	candidates []*agentgateway.AgentgatewayPolicy,
	group, kind, name string,
	matcher targetSectionMatcher,
) *agentgateway.AgentgatewayPolicy {
	var (
		selected *agentgateway.AgentgatewayPolicy
		bestRank sectionMatchRank
	)
	for _, candidate := range candidates {
		rank := bestMatchingPolicyTargetRank(candidate.Spec.TargetRefs, group, kind, name, matcher)
		if rank == sectionNoMatch {
			continue
		}
		if selected == nil || rank > bestRank || (rank == bestRank && higherPriority(candidate, selected)) {
			selected = candidate
			bestRank = rank
		}
	}
	return selected
}

func bestMatchingBackendTLSPolicy(
	candidates []*gwv1.BackendTLSPolicy,
	kind, name string,
	matcher targetSectionMatcher,
) *gwv1.BackendTLSPolicy {
	var (
		selected *gwv1.BackendTLSPolicy
		bestRank sectionMatchRank
	)
	for _, candidate := range candidates {
		rank := bestMatchingBackendTLSTargetRank(candidate.Spec.TargetRefs, kind, name, matcher)
		if rank == sectionNoMatch {
			continue
		}
		if selected == nil || rank > bestRank || (rank == bestRank && higherPriority(candidate, selected)) {
			selected = candidate
			bestRank = rank
		}
	}
	return selected
}

func bestMatchingPolicyTargetRank(
	targetRefs []shared.LocalPolicyTargetReferenceWithSectionName,
	group, kind, name string,
	matcher targetSectionMatcher,
) sectionMatchRank {
	best := sectionNoMatch
	for _, targetRef := range targetRefs {
		if string(targetRef.Group) != group || string(targetRef.Kind) != kind || string(targetRef.Name) != name {
			continue
		}
		if rank := matcher.match(targetRef.SectionName); rank > best {
			best = rank
		}
	}
	return best
}

func bestMatchingBackendTLSTargetRank(
	targetRefs []gwv1.LocalPolicyTargetReferenceWithSectionName,
	kind, name string,
	matcher targetSectionMatcher,
) sectionMatchRank {
	best := sectionNoMatch
	for _, targetRef := range targetRefs {
		if string(targetRef.Kind) != kind || string(targetRef.Name) != name {
			continue
		}
		if rank := matcher.match(targetRef.SectionName); rank > best {
			best = rank
		}
	}
	return best
}

func (m targetSectionMatcher) match(sectionName *gwv1.SectionName) sectionMatchRank {
	if sectionName == nil {
		if m.allowWhole {
			return sectionWholeResourceMatch
		}
		return sectionNoMatch
	}
	if slices.Contains(m.exact, string(*sectionName)) {
		return sectionExactMatch
	}
	return sectionNoMatch
}

func higherPriority(a, b metav1.Object) bool {
	ts := a.GetCreationTimestamp().Compare(b.GetCreationTimestamp().Time)
	if ts < 0 {
		return true
	}
	if ts > 0 {
		return false
	}
	if a.GetNamespace() != b.GetNamespace() {
		return a.GetNamespace() < b.GetNamespace()
	}
	return a.GetName() < b.GetName()
}

func dedupeStrings(values []string) []string {
	if len(values) == 0 {
		return nil
	}
	seen := make(map[string]struct{}, len(values))
	deduped := make([]string, 0, len(values))
	for _, value := range values {
		if value == "" {
			continue
		}
		if _, ok := seen[value]; ok {
			continue
		}
		seen[value] = struct{}{}
		deduped = append(deduped, value)
	}
	return deduped
}
