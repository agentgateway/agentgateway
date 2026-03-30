package remotehttp

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

type resolvedTLS struct {
	tlsConfig    *tls.Config
	verification agentgateway.InsecureTLSMode
	serverName   string
	caBundleHash string
	nextProtos   []string
}

func (r *defaultResolver) resolveTLS(
	krtctx krt.HandlerContext,
	namespace, group, kind, name string,
	agwMatcher targetSectionMatcher,
	backendTLSMatcher targetSectionMatcher,
	backendPolicies *agentgateway.BackendFull,
) (*resolvedTLS, error) {
	if backendPolicies != nil && backendPolicies.TLS != nil {
		return resolvedTLSFromBackendTLS(krtctx, r.cfgmaps, namespace, backendPolicies.TLS)
	}
	if agwPolicy := r.fetchBestMatchingAgentgatewayPolicy(krtctx, namespace, group, kind, name, agwMatcher); agwPolicy != nil && agwPolicy.Spec.Backend != nil && agwPolicy.Spec.Backend.TLS != nil {
		return resolvedTLSFromBackendTLS(krtctx, r.cfgmaps, namespace, agwPolicy.Spec.Backend.TLS)
	}
	if backendTLSPolicy := r.fetchBestMatchingBackendTLSPolicy(krtctx, namespace, group, kind, name, backendTLSMatcher); backendTLSPolicy != nil {
		return resolvedTLSFromBackendTLSPolicy(krtctx, r.cfgmaps, namespace, backendTLSPolicy)
	}
	return nil, nil
}

func resolvedTLSFromBackendTLSPolicy(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	namespace string,
	policy *gwv1.BackendTLSPolicy,
) (*resolvedTLS, error) {
	tlsConfig := &tls.Config{
		MinVersion: tls.VersionTLS12,
		ServerName: string(policy.Spec.Validation.Hostname),
	}

	resolved := &resolvedTLS{
		tlsConfig:    tlsConfig,
		verification: "",
		serverName:   tlsConfig.ServerName,
	}

	if len(policy.Spec.Validation.CACertificateRefs) == 0 {
		return resolved, nil
	}

	rootCAs, caBundleHash, err := caBundleFromGatewayRefs(krtctx, cfgmaps, namespace, policy.Spec.Validation.CACertificateRefs)
	if err != nil {
		return nil, err
	}
	tlsConfig.RootCAs = rootCAs
	resolved.caBundleHash = caBundleHash
	return resolved, nil
}

func resolvedTLSFromBackendTLS(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	namespace string,
	btls *agentgateway.BackendTLS,
) (*resolvedTLS, error) {
	if btls == nil {
		return nil, nil
	}

	resolved := &resolvedTLS{
		verification: verificationMode(btls.InsecureSkipVerify),
		serverName:   ptr.OrEmpty(btls.Sni),
		nextProtos:   ptr.OrEmpty(btls.AlpnProtocols),
	}
	resolved.tlsConfig = &tls.Config{
		ServerName: resolved.serverName,
		MinVersion: tls.VersionTLS12,
		NextProtos: resolved.nextProtos,
	}

	if len(btls.CACertificateRefs) > 0 {
		rootCAs, caBundleHash, err := caBundleFromConfigMaps(krtctx, cfgmaps, namespace, btls.CACertificateRefs)
		if err != nil {
			return nil, err
		}
		resolved.tlsConfig.RootCAs = rootCAs
		resolved.caBundleHash = caBundleHash
	}

	switch resolved.verification {
	case agentgateway.InsecureTLSModeAll:
		resolved.tlsConfig.InsecureSkipVerify = true //nolint:gosec
	case agentgateway.InsecureTLSModeHostname:
		resolved.tlsConfig.InsecureSkipVerify = true //nolint:gosec
		resolved.tlsConfig.VerifyConnection = verifyPeerChainWithoutHostname(resolved.tlsConfig.RootCAs)
	}

	return resolved, nil
}

func verificationMode(mode *agentgateway.InsecureTLSMode) agentgateway.InsecureTLSMode {
	switch ptr.OrDefault(mode, "") {
	case agentgateway.InsecureTLSModeAll:
		return agentgateway.InsecureTLSModeAll
	case agentgateway.InsecureTLSModeHostname:
		return agentgateway.InsecureTLSModeHostname
	default:
		return ""
	}
}

func verifyPeerChainWithoutHostname(rootCAs *x509.CertPool) func(tls.ConnectionState) error {
	return func(cs tls.ConnectionState) error {
		if len(cs.PeerCertificates) == 0 {
			return fmt.Errorf("jwks endpoint did not present a peer certificate")
		}

		intermediates := x509.NewCertPool()
		for _, cert := range cs.PeerCertificates[1:] {
			intermediates.AddCert(cert)
		}

		opts := x509.VerifyOptions{
			Roots:         rootCAs,
			Intermediates: intermediates,
			KeyUsages:     []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth},
		}
		_, err := cs.PeerCertificates[0].Verify(opts)
		return err
	}
}
