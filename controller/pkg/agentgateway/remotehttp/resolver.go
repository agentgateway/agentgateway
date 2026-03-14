package remotehttp

import (
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	krtpkg "github.com/agentgateway/agentgateway/controller/pkg/utils/krtutil"
)

type Inputs struct {
	ConfigMaps           krt.Collection[*corev1.ConfigMap]
	Services             krt.Collection[*corev1.Service]
	Backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	AgentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	BackendTLSPolicies   krt.Collection[*gwv1.BackendTLSPolicy]
}

type ResolveInput struct {
	ParentName       string
	DefaultNamespace string
	BackendRef       gwv1.BackendObjectReference
	Path             string
	DefaultPort      string
}

type Resolver interface {
	Resolve(krtctx krt.HandlerContext, input ResolveInput) (*ResolvedEndpoint, error)
}

type defaultResolver struct {
	cfgmaps              krt.Collection[*corev1.ConfigMap]
	services             krt.Collection[*corev1.Service]
	backends             krt.Collection[*agentgateway.AgentgatewayBackend]
	agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]
	backendTLSPolicies   krt.Collection[*gwv1.BackendTLSPolicy]
	policiesByTargetRef  krt.Index[policyTargetRefKey, *agentgateway.AgentgatewayPolicy]
	backendTLSByTarget   krt.Index[backendTLSPolicyTargetRefKey, *gwv1.BackendTLSPolicy]
}

func NewResolver(inputs Inputs) Resolver {
	return &defaultResolver{
		cfgmaps:              inputs.ConfigMaps,
		services:             inputs.Services,
		backends:             inputs.Backends,
		agentgatewayPolicies: inputs.AgentgatewayPolicies,
		backendTLSPolicies:   inputs.BackendTLSPolicies,
		policiesByTargetRef:  newPolicyTargetRefIndex(inputs.AgentgatewayPolicies),
		backendTLSByTarget:   newBackendTLSPolicyTargetRefIndex(inputs.BackendTLSPolicies),
	}
}

func (r *defaultResolver) Resolve(krtctx krt.HandlerContext, input ResolveInput) (*ResolvedEndpoint, error) {
	path := strings.TrimPrefix(input.Path, "/")
	resolved, err := r.resolveConnection(krtctx, input.ParentName, input.DefaultNamespace, input.BackendRef, input.DefaultPort)
	if err != nil {
		return nil, err
	}

	request := Request{}
	if resolved.tls == nil {
		request.URL = fmt.Sprintf("http://%s/%s", resolved.connectHost, path)
		return &ResolvedEndpoint{
			Key:     request.Key(),
			Request: request,
		}, nil
	}

	request.URL = fmt.Sprintf("https://%s/%s", resolved.connectHost, path)
	request.Transport = TransportFingerprint{
		Verification: resolved.tls.verification,
		ServerName:   resolved.tls.serverName,
		CABundleHash: resolved.tls.caBundleHash,
		NextProtos:   append([]string(nil), resolved.tls.nextProtos...),
	}

	return &ResolvedEndpoint{
		Key:       request.Key(),
		Request:   request,
		TLSConfig: resolved.tls.tlsConfig,
	}, nil
}

func newPolicyTargetRefIndex(agentgatewayPolicies krt.Collection[*agentgateway.AgentgatewayPolicy]) krt.Index[policyTargetRefKey, *agentgateway.AgentgatewayPolicy] {
	return krtpkg.UnnamedIndex(agentgatewayPolicies, func(in *agentgateway.AgentgatewayPolicy) []policyTargetRefKey {
		keys := make([]policyTargetRefKey, 0, len(in.Spec.TargetRefs))
		for _, ref := range in.Spec.TargetRefs {
			keys = append(keys, policyTargetRefKey{
				Name:      string(ref.Name),
				Kind:      string(ref.Kind),
				Group:     string(ref.Group),
				Namespace: in.Namespace,
			})
		}
		return keys
	})
}

func newBackendTLSPolicyTargetRefIndex(backendTLSPolicies krt.Collection[*gwv1.BackendTLSPolicy]) krt.Index[backendTLSPolicyTargetRefKey, *gwv1.BackendTLSPolicy] {
	return krtpkg.UnnamedIndex(backendTLSPolicies, func(in *gwv1.BackendTLSPolicy) []backendTLSPolicyTargetRefKey {
		keys := make([]backendTLSPolicyTargetRefKey, 0, len(in.Spec.TargetRefs))
		for _, ref := range in.Spec.TargetRefs {
			keys = append(keys, backendTLSPolicyTargetRefKey{
				Group:     string(ref.Group),
				Name:      string(ref.Name),
				Kind:      string(ref.Kind),
				Namespace: in.Namespace,
			})
		}
		return keys
	})
}
