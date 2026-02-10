package plugins

import (
	"errors"
	"fmt"

	"github.com/agentgateway/agentgateway/go/api"
	"google.golang.org/protobuf/types/known/durationpb"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
)

const (
	frontendTcpPolicySuffix     = ":frontend-tcp"
	frontendTlsPolicySuffix     = ":frontend-tls"
	frontendHttpPolicySuffix    = ":frontend-http"
	frontendLoggingPolicySuffix = ":frontend-logging"
	frontendTracingPolicySuffix = ":frontend-tracing"
)

func translateFrontendPolicyToAgw(
	policyCtx PolicyCtx,
	policy *agentgateway.AgentgatewayPolicy,
	policyTarget *api.PolicyTarget,
) ([]AgwPolicy, error) {
	frontend := policy.Spec.Frontend
	if frontend == nil {
		return nil, nil
	}
	agwPolicies := make([]AgwPolicy, 0)
	var errs []error

	policyName := getFrontendPolicyName(policy.Namespace, policy.Name)

	if s := frontend.HTTP; s != nil {
		pol := translateFrontendHTTP(policy, policyName, policyTarget)
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := frontend.TLS; s != nil {
		pol := translateFrontendTLS(policy, policyName, policyTarget)
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := frontend.TCP; s != nil {
		pol := translateFrontendTCP(policy, policyName, policyTarget)
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := frontend.AccessLog; s != nil {
		pol := translateFrontendAccessLog(policy, policyName, policyTarget)
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := frontend.Tracing; s != nil {
		pol, err := translateFrontendTracing(policyCtx, policy, policyName, policyTarget)
		if err != nil {
			logger.Error("error processing tracing", "err", err)
			errs = append(errs, err)
		}
		agwPolicies = append(agwPolicies, pol...)
	}

	return agwPolicies, errors.Join(errs...)
}

func translateFrontendTracing(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, name string, target *api.PolicyTarget) ([]AgwPolicy, error) {
	tracing := policy.Spec.Frontend.Tracing
	if tracing == nil {
		return nil, nil
	}

	provider, err := buildBackendRef(ctx, tracing.BackendRef, policy.Namespace)
	if err != nil {
		return nil, fmt.Errorf("failed to translate tracing backend ref: %v", err)
	}

	var addAttributes []*api.FrontendPolicySpec_TracingAttribute
	var rmAttributes []string
	if tracing.Attributes != nil {
		for _, add := range tracing.Attributes.Add {
			addAttributes = append(addAttributes, &api.FrontendPolicySpec_TracingAttribute{
				Name:  add.Name,
				Value: string(add.Expression),
			})
		}
		for _, rm := range tracing.Attributes.Remove {
			rmAttributes = append(rmAttributes, rm)
		}
	}

	var addResources []*api.FrontendPolicySpec_TracingAttribute
	if tracing.Resources != nil {
		for _, add := range tracing.Resources {
			addResources = append(addResources, &api.FrontendPolicySpec_TracingAttribute{
				Name:  add.Name,
				Value: string(add.Expression),
			})
		}
	}

	var randomSampling *string
	if tracing.RandomSampling != nil {
		randomSampling = ptr.Of(string(*tracing.RandomSampling))
	}

	var clientSampling *string
	if tracing.ClientSampling != nil {
		clientSampling = ptr.Of(string(*tracing.ClientSampling))
	}

	var protocol api.FrontendPolicySpec_Tracing_Protocol
	switch tracing.Protocol {
	case agentgateway.TracingProtocolGrpc:
		protocol = api.FrontendPolicySpec_Tracing_GRPC
	case agentgateway.TracingProtocolHttp:
		protocol = api.FrontendPolicySpec_Tracing_HTTP
	default:
		// default to HTTP
		protocol = api.FrontendPolicySpec_Tracing_HTTP
	}

	tracingPolicy := &api.Policy{
		Key:    name + frontendTracingPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_Tracing_{Tracing: &api.FrontendPolicySpec_Tracing{
					ProviderBackend: provider,
					Attributes:      addAttributes,
					Remove:          rmAttributes,
					Resources:       addResources,
					Protocol:        protocol,
					RandomSampling:  randomSampling,
					ClientSampling:  clientSampling,
				}},
			},
		},
	}

	logger.Debug("generated tracing policy",
		"policy", policy.Name,
		"agentgateway_policy", tracingPolicy.Name,
		"target", target)

	return []AgwPolicy{{Policy: tracingPolicy}}, nil
}

func translateFrontendAccessLog(policy *agentgateway.AgentgatewayPolicy, name string, target *api.PolicyTarget) []AgwPolicy {
	logging := policy.Spec.Frontend.AccessLog
	spec := &api.FrontendPolicySpec_Logging{}
	if f := logging.Filter; f != nil {
		spec.Filter = (*string)(f)
	}
	if a := logging.Attributes; a != nil {
		f := &api.FrontendPolicySpec_Logging_Fields{
			Remove: a.Remove,
			Add: slices.Map(a.Add, func(e agentgateway.AttributeAdd) *api.FrontendPolicySpec_Logging_Field {
				return &api.FrontendPolicySpec_Logging_Field{
					Name:       e.Name,
					Expression: string(e.Expression),
				}
			}),
		}
		spec.Fields = f
	}

	loggingPolicy := &api.Policy{
		Key:    name + frontendLoggingPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_Logging_{
					Logging: spec,
				},
			},
		},
	}

	logger.Debug("generated logging policy",
		"policy", policy.Name,
		"agentgateway_policy", loggingPolicy.Name,
		"target", target)

	return []AgwPolicy{{Policy: loggingPolicy}}
}

func translateFrontendTCP(policy *agentgateway.AgentgatewayPolicy, name string, target *api.PolicyTarget) []AgwPolicy {
	tcp := policy.Spec.Frontend.TCP
	spec := &api.FrontendPolicySpec_TCP{}
	if ka := tcp.KeepAlive; ka != nil {
		spec.Keepalives = &api.KeepaliveConfig{}
		if ka.Time != nil {
			spec.Keepalives.Time = durationpb.New(ka.Time.Duration)
		}
		if ka.Interval != nil {
			spec.Keepalives.Interval = durationpb.New(ka.Interval.Duration)
		}
		if ka.Retries != nil {
			spec.Keepalives.Retries = castUint32(ka.Retries) //nolint:gosec // G115: kubebuilder validation ensures safe for uint32
		}
	}

	tcpPolicy := &api.Policy{
		Key:    name + frontendTcpPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_Tcp{
					Tcp: spec,
				},
			},
		},
	}

	logger.Debug("generated tcp policy",
		"policy", policy.Name,
		"agentgateway_policy", tcpPolicy.Name,
		"target", target)

	return []AgwPolicy{{Policy: tcpPolicy}}
}

func castUint32[T ~int32](ka *T) *uint32 {
	return ptr.Of((uint32)(*ka))
}

func translateFrontendTLS(policy *agentgateway.AgentgatewayPolicy, name string, target *api.PolicyTarget) []AgwPolicy {
	tls := policy.Spec.Frontend.TLS
	spec := &api.FrontendPolicySpec_TLS{}
	if ka := tls.HandshakeTimeout; ka != nil {
		spec.HandshakeTimeout = durationpb.New(ka.Duration)
	}

	if tls.AlpnProtocols != nil {
		spec.Alpn = &api.Alpn{Protocols: *tls.AlpnProtocols}
	}

	if tls.MaxTLSVersion != nil {
		switch *tls.MaxTLSVersion {
		case agentgateway.TLSVersion1_2:
			spec.MaxVersion = ptr.Of(api.TLSConfig_TLS_V1_2)
		case agentgateway.TLSVersion1_3:
			spec.MaxVersion = ptr.Of(api.TLSConfig_TLS_V1_3)
		default:
			logger.Warn("unknown tls version for max", "version", tls.MaxTLSVersion)
			spec.MaxVersion = nil
		}
	}

	if tls.MinTLSVersion != nil {
		switch *tls.MinTLSVersion {
		case agentgateway.TLSVersion1_2:
			spec.MinVersion = ptr.Of(api.TLSConfig_TLS_V1_2)
		case agentgateway.TLSVersion1_3:
			spec.MinVersion = ptr.Of(api.TLSConfig_TLS_V1_3)
		default:
			logger.Warn("unknown tls version for min", "version", tls.MinTLSVersion)
			spec.MinVersion = nil
		}
	}

	var agwCipherSuites []api.TLSConfig_CipherSuite
	for _, cs := range tls.CipherSuites {
		switch cs {
		case agentgateway.CipherSuiteTLS13_AES_256_GCM_SHA384:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_AES_256_GCM_SHA384)
		case agentgateway.CipherSuiteTLS13_AES_128_GCM_SHA256:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_AES_128_GCM_SHA256)
		case agentgateway.CipherSuiteTLS13_CHACHA20_POLY1305_SHA256:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_CHACHA20_POLY1305_SHA256)
		case agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384)
		case agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256)
		case agentgateway.CipherSuiteTLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256)
		case agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384)
		case agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256)
		case agentgateway.CipherSuiteTLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256:
			agwCipherSuites = append(agwCipherSuites, api.TLSConfig_TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256)
		default:
			logger.Warn("unknown tls cipher suite", "cipher_suite", cs)
			continue
		}
	}
	if len(agwCipherSuites) > 0 {
		spec.CipherSuites = agwCipherSuites
	}

	tlsPolicy := &api.Policy{
		Key:    name + frontendTlsPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_Tls{
					Tls: spec,
				},
			},
		},
	}

	logger.Debug("generated tls policy",
		"policy", policy.Name,
		"agentgateway_policy", tlsPolicy.Name,
		"target", target)

	return []AgwPolicy{{Policy: tlsPolicy}}
}

func translateFrontendHTTP(policy *agentgateway.AgentgatewayPolicy, name string, target *api.PolicyTarget) []AgwPolicy {
	http := policy.Spec.Frontend.HTTP
	spec := &api.FrontendPolicySpec_HTTP{}
	if v := http.MaxBufferSize; v != nil {
		spec.MaxBufferSize = castUint32(v) //nolint:gosec // G115: kubebuilder validation ensures safe for uint32
	}
	if v := http.HTTP1MaxHeaders; v != nil {
		spec.Http1MaxHeaders = castUint32(v) //nolint:gosec // G115: kubebuilder validation ensures safe for uint32
	}
	if v := http.HTTP1IdleTimeout; v != nil {
		spec.Http1IdleTimeout = durationpb.New(v.Duration)
	}
	if v := http.HTTP2WindowSize; v != nil {
		spec.Http2WindowSize = castUint32(v) //nolint:gosec // G115: kubebuilder validation ensures safe for uint32
	}
	if v := http.HTTP2ConnectionWindowSize; v != nil {
		spec.Http2ConnectionWindowSize = castUint32(v) //nolint:gosec // G115: kubebuilder validation ensures safe for uint32
	}
	if v := http.HTTP2FrameSize; v != nil {
		spec.Http2FrameSize = castUint32(v) //nolint:gosec // G115: kubebuilder validation ensures safe for uint32
	}
	if v := http.HTTP2KeepaliveInterval; v != nil {
		spec.Http2KeepaliveInterval = durationpb.New(v.Duration)
	}
	if v := http.HTTP2KeepaliveTimeout; v != nil {
		spec.Http2KeepaliveTimeout = durationpb.New(v.Duration)
	}

	httpPolicy := &api.Policy{
		Key:    name + frontendHttpPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_Http{
					Http: spec,
				},
			},
		},
	}

	logger.Debug("generated http policy",
		"policy", policy.Name,
		"agentgateway_policy", httpPolicy.Name,
		"target", target)

	return []AgwPolicy{{Policy: httpPolicy}}
}
