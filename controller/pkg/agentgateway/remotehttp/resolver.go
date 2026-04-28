package remotehttp

import (
	"crypto/tls"
	"fmt"
	"strings"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/policyselection"
)

type Inputs struct {
	ConfigMaps     krt.Collection[*corev1.ConfigMap]
	Services       krt.Collection[*corev1.Service]
	Backends       krt.Collection[*agentgateway.AgentgatewayBackend]
	PolicySelector policyselection.Selector
}

type ResolveInput struct {
	ParentName       string
	DefaultNamespace string
	BackendRef       gwv1.BackendObjectReference
	Path             string
	DefaultPort      string
}

type Resolver interface {
	Resolve(krtctx krt.HandlerContext, input ResolveInput) (*ResolvedTarget, error)
}

type defaultResolver struct {
	cfgmaps        krt.Collection[*corev1.ConfigMap]
	services       krt.Collection[*corev1.Service]
	backends       krt.Collection[*agentgateway.AgentgatewayBackend]
	policySelector policyselection.Selector
}

func NewResolver(inputs Inputs) Resolver {
	return &defaultResolver{
		cfgmaps:        inputs.ConfigMaps,
		services:       inputs.Services,
		backends:       inputs.Backends,
		policySelector: inputs.PolicySelector,
	}
}

func (r *defaultResolver) Resolve(krtctx krt.HandlerContext, input ResolveInput) (*ResolvedTarget, error) {
	path := strings.TrimPrefix(input.Path, "/")
	resolved, err := r.resolveConnection(krtctx, input.ParentName, input.DefaultNamespace, input.BackendRef, input.DefaultPort)
	if err != nil {
		return nil, err
	}

	target := FetchTarget{
		ProxyURL: resolved.proxyURL,
	}

	if resolved.proxyTLS != nil {
		target.ProxyTransport = TransportFingerprint{
			Verification: resolved.proxyTLS.verification,
			ServerName:   resolved.proxyTLS.serverName,
			CABundleHash: resolved.proxyTLS.caBundleHash,
			NextProtos:   append([]string(nil), resolved.proxyTLS.nextProtos...),
		}
	}

	var proxyTLSConfig *tls.Config
	if resolved.proxyTLS != nil {
		proxyTLSConfig = resolved.proxyTLS.tlsConfig
	}

	if resolved.tls == nil {
		target.URL = fmt.Sprintf("http://%s/%s", resolved.connectHost, path)
		return &ResolvedTarget{
			Key:            target.Key(),
			Target:         target,
			ProxyTLSConfig: proxyTLSConfig,
		}, nil
	}

	target.URL = fmt.Sprintf("https://%s/%s", resolved.connectHost, path)
	target.Transport = TransportFingerprint{
		Verification: resolved.tls.verification,
		ServerName:   resolved.tls.serverName,
		CABundleHash: resolved.tls.caBundleHash,
		NextProtos:   append([]string(nil), resolved.tls.nextProtos...),
	}

	return &ResolvedTarget{
		Key:            target.Key(),
		Target:         target,
		TLSConfig:      resolved.tls.tlsConfig,
		ProxyTLSConfig: proxyTLSConfig,
	}, nil
}
