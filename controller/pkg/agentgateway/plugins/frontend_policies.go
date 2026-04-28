package plugins

import (
	"errors"
	"fmt"

	"google.golang.org/protobuf/types/known/durationpb"
	"istio.io/istio/pkg/ptr"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const (
	frontendTcpPolicySuffix     = ":frontend-tcp"
	frontendNetworkAuthzSuffix  = ":frontend-network-authz"
	frontendTlsPolicySuffix     = ":frontend-tls"
	frontendHttpPolicySuffix    = ":frontend-http"
	frontendProxyPolicySuffix   = ":frontend-proxy"
	frontendLoggingPolicySuffix = ":frontend-logging"
	frontendTracingPolicySuffix = ":frontend-tracing"
	frontendMetricsPolicySuffix = ":frontend-metrics"
)

func translateFrontendPolicyToAgw(
	policyCtx PolicyCtx,
	policy *agentgateway.AgentgatewayPolicy,
) ([]*api.Policy, error) {
	frontend := policy.Spec.Frontend
	if frontend == nil {
		return nil, nil
	}
	agwPolicies := make([]*api.Policy, 0)
	var errs []error

	policyName := getFrontendPolicyName(policy.Namespace, policy.Name)
	appendPolicy := func(kind string) func(*api.Policy, error) {
		return func(p *api.Policy, err error) {
			if err != nil {
				name := fmt.Sprintf("%s %s", kind, policyName)
				logger.Error("error processing policy", "policy", name, "error", err)
				errs = append(errs, err)
			}
			if p != nil {
				agwPolicies = append(agwPolicies, p)
			}
		}
	}

	if s := frontend.HTTP; s != nil {
		appendPolicy("http")(translateFrontendHTTP(policy, policyName), nil)
	}

	if s := frontend.ProxyProtocol; s != nil {
		appendPolicy("proxyProtocol")(translateFrontendProxyProtocol(policy, policyName), nil)
	}

	if s := frontend.TLS; s != nil {
		appendPolicy("tls")(translateFrontendTLS(policy, policyName), nil)
	}

	if s := frontend.TCP; s != nil {
		appendPolicy("tcp")(translateFrontendTCP(policy, policyName), nil)
	}

	if s := frontend.NetworkAuthorization; s != nil {
		appendPolicy("networkAuthorization")(translateFrontendNetworkAuthorization(policy, policyName), nil)
	}

	if s := frontend.AccessLog; s != nil {
		appendPolicy("accessLog")(translateFrontendAccessLog(policyCtx, policy, policyName))
	}

	if s := frontend.Tracing; s != nil {
		appendPolicy("tracing")(translateFrontendTracing(policyCtx, policy, policyName))
	}

	if s := frontend.Metrics; s != nil {
		appendPolicy("metrics")(translateFrontendMetrics(policy, policyName))
	}

	return agwPolicies, errors.Join(errs...)
}

func translateFrontendTracing(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, name string) (*api.Policy, error) {
	tracing := policy.Spec.Frontend.Tracing
	var errs []error
	provider, err := buildBackendRef(ctx, tracing.BackendRef, policy.Namespace)
	if err != nil {
		errs = append(errs, fmt.Errorf("failed to translate tracing backend ref: %v", err))
	}

	var addAttributes []*api.FrontendPolicySpec_TracingAttribute
	var rmAttributes []string
	if tracing.Attributes != nil {
		for _, add := range tracing.Attributes.Add {
			if !isCEL(add.Expression) {
				errs = append(errs, fmt.Errorf("frontend tracing attribute %q is not a valid CEL expression: %s", add.Name, add.Expression))
			}
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
			if !isCEL(add.Expression) {
				errs = append(errs, fmt.Errorf("frontend tracing resource %q is not a valid CEL expression: %s", add.Name, add.Expression))
			}
			addResources = append(addResources, &api.FrontendPolicySpec_TracingAttribute{
				Name:  add.Name,
				Value: string(add.Expression),
			})
		}
	}

	var randomSampling *string
	if tracing.RandomSampling != nil {
		randomSampling = castCELPtr(tracing.RandomSampling, func(expr shared.CELExpression) {
			errs = append(errs, fmt.Errorf("frontend tracing randomSampling is not a valid CEL expression: %s", expr))
		})
	}

	var clientSampling *string
	if tracing.ClientSampling != nil {
		clientSampling = castCELPtr(tracing.ClientSampling, func(expr shared.CELExpression) {
			errs = append(errs, fmt.Errorf("frontend tracing clientSampling is not a valid CEL expression: %s", expr))
		})
	}

	var path *string
	if tracing.Path != nil {
		path = new(*tracing.Path)
	}

	var protocol api.FrontendPolicySpec_Tracing_Protocol
	switch tracing.Protocol {
	case agentgateway.OTLPProtocolGrpc:
		protocol = api.FrontendPolicySpec_Tracing_GRPC
	case agentgateway.OTLPProtocolHttp:
		protocol = api.FrontendPolicySpec_Tracing_HTTP
	default:
		// default to HTTP
		protocol = api.FrontendPolicySpec_Tracing_GRPC
	}

	tracingPolicy := &api.Policy{
		Key:  name + frontendTracingPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_Tracing_{Tracing: &api.FrontendPolicySpec_Tracing{
					ProviderBackend: provider,
					Attributes:      addAttributes,
					Remove:          rmAttributes,
					Resources:       addResources,
					Protocol:        protocol,
					Path:            path,
					RandomSampling:  randomSampling,
					ClientSampling:  clientSampling,
				}},
			},
		},
	}

	logger.Debug("generated tracing policy",
		"policy", policy.Name,
		"agentgateway_policy", tracingPolicy.Name)

	return tracingPolicy, errors.Join(errs...)
}

func translateFrontendAccessLog(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, name string) (*api.Policy, error) {
	logging := policy.Spec.Frontend.AccessLog
	spec := &api.FrontendPolicySpec_Logging{}
	var errs []error
	if f := logging.Filter; f != nil {
		spec.Filter = castCELPtr(f, func(expr shared.CELExpression) {
			errs = append(errs, fmt.Errorf("frontend accessLog filter is not a valid CEL expression: %s", expr))
		})
	}
	if a := logging.Attributes; a != nil {
		fields := make([]*api.FrontendPolicySpec_Logging_Field, 0, len(a.Add))
		for _, add := range a.Add {
			if !isCEL(add.Expression) {
				errs = append(errs, fmt.Errorf("frontend accessLog field %q is not a valid CEL expression: %s", add.Name, add.Expression))
			}
			fields = append(fields, &api.FrontendPolicySpec_Logging_Field{
				Name:       add.Name,
				Expression: string(add.Expression),
			})
		}
		f := &api.FrontendPolicySpec_Logging_Fields{
			Remove: a.Remove,
			Add:    fields,
		}
		spec.Fields = f
	}
	if otlp := logging.Otlp; otlp != nil {
		provider, err := buildBackendRef(ctx, otlp.BackendRef, policy.Namespace)
		if err != nil {
			errs = append(errs, fmt.Errorf("failed to translate access log OTLP backend ref: %v", err))
		}

		var protocol api.FrontendPolicySpec_Logging_OtlpAccessLog_Protocol
		switch otlp.Protocol {
		case agentgateway.OTLPProtocolGrpc:
			protocol = api.FrontendPolicySpec_Logging_OtlpAccessLog_GRPC
		case agentgateway.OTLPProtocolHttp:
			protocol = api.FrontendPolicySpec_Logging_OtlpAccessLog_HTTP
		default:
			protocol = api.FrontendPolicySpec_Logging_OtlpAccessLog_GRPC
		}

		var path *string
		if otlp.Path != nil {
			path = new(*otlp.Path)
		}

		spec.OtlpAccessLog = &api.FrontendPolicySpec_Logging_OtlpAccessLog{
			ProviderBackend: provider,
			Protocol:        protocol,
			Path:            path,
		}
	}

	loggingPolicy := &api.Policy{
		Key:  name + frontendLoggingPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
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
		"agentgateway_policy", loggingPolicy.Name)

	return loggingPolicy, errors.Join(errs...)
}

func translateFrontendTCP(policy *agentgateway.AgentgatewayPolicy, name string) *api.Policy {
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
		Key:  name + frontendTcpPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
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
		"agentgateway_policy", tcpPolicy.Name)

	return tcpPolicy
}

func translateFrontendProxyProtocol(policy *agentgateway.AgentgatewayPolicy, name string) *api.Policy {
	proxy := policy.Spec.Frontend.ProxyProtocol
	version := api.FrontendPolicySpec_ProxyProtocol_V2
	switch proxy.Version {
	case agentgateway.ProxyProtocolVersionV1:
		version = api.FrontendPolicySpec_ProxyProtocol_V1
	case agentgateway.ProxyProtocolVersionAll:
		version = api.FrontendPolicySpec_ProxyProtocol_ALL
	case agentgateway.ProxyProtocolVersionV2, "":
		version = api.FrontendPolicySpec_ProxyProtocol_V2
	default:
		logger.Warn("unknown proxy protocol version", "version", proxy.Version)
	}

	mode := api.FrontendPolicySpec_ProxyProtocol_STRICT
	switch proxy.Mode {
	case agentgateway.ProxyProtocolModeOptional:
		mode = api.FrontendPolicySpec_ProxyProtocol_OPTIONAL
	case agentgateway.ProxyProtocolModeStrict, "":
		mode = api.FrontendPolicySpec_ProxyProtocol_STRICT
	default:
		logger.Warn("unknown proxy protocol mode", "mode", proxy.Mode)
	}

	proxyPolicy := &api.Policy{
		Key:  name + frontendProxyPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_ProxyProtocol_{
					ProxyProtocol: &api.FrontendPolicySpec_ProxyProtocol{
						Version: version,
						Mode:    mode,
					},
				},
			},
		},
	}

	logger.Debug("generated proxy policy",
		"policy", policy.Name,
		"agentgateway_policy", proxyPolicy.Name)

	return proxyPolicy
}

func translateFrontendNetworkAuthorization(policy *agentgateway.AgentgatewayPolicy, name string) *api.Policy {
	auth := policy.Spec.Frontend.NetworkAuthorization
	var allowPolicies, denyPolicies, requirePolicies []string
	if auth.Action == shared.AuthorizationPolicyActionDeny {
		denyPolicies = append(denyPolicies, cast(auth.Policy.MatchExpressions)...)
	} else if auth.Action == shared.AuthorizationPolicyActionRequire {
		requirePolicies = append(requirePolicies, cast(auth.Policy.MatchExpressions)...)
	} else {
		allowPolicies = append(allowPolicies, cast(auth.Policy.MatchExpressions)...)
	}

	pol := &api.Policy{
		Key:  name + frontendNetworkAuthzSuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_NetworkAuthorization_{
					NetworkAuthorization: &api.FrontendPolicySpec_NetworkAuthorization{
						Allow:   allowPolicies,
						Deny:    denyPolicies,
						Require: requirePolicies,
					},
				},
			},
		},
	}

	logger.Debug("generated frontend network authorization policy",
		"policy", policy.Name,
		"agentgateway_policy", pol.Name)

	return pol
}

func castUint32[T ~int32](ka *T) *uint32 {
	return new((uint32)(*ka))
}

func translateFrontendTLS(policy *agentgateway.AgentgatewayPolicy, name string) *api.Policy {
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
		Key:  name + frontendTlsPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
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
		"agentgateway_policy", tlsPolicy.Name)

	return tlsPolicy
}

func translateFrontendHTTP(policy *agentgateway.AgentgatewayPolicy, name string) *api.Policy {
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
	if v := http.MaxConnectionDuration; v != nil {
		spec.MaxConnectionDuration = durationpb.New(v.Duration)
	}

	httpPolicy := &api.Policy{
		Key:  name + frontendHttpPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
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
		"agentgateway_policy", httpPolicy.Name)

	return httpPolicy
}

func translateFrontendMetrics(policy *agentgateway.AgentgatewayPolicy, name string) (*api.Policy, error) {
	metricsSpec := policy.Spec.Frontend.Metrics
	spec := &api.FrontendPolicySpec_Metrics{}
	var errs []error

	fields := make([]*api.FrontendPolicySpec_Metrics_Field, 0, len(metricsSpec.Attributes.Add))
	for _, add := range metricsSpec.Attributes.Add {
		if !isCEL(add.Expression) {
			errs = append(errs, fmt.Errorf("frontend metrics field %q is not a valid CEL expression: %s", add.Name, add.Expression))
		}
		fields = append(fields, &api.FrontendPolicySpec_Metrics_Field{
			Name:       add.Name,
			Expression: string(add.Expression),
		})
	}
	spec.Fields = &api.FrontendPolicySpec_Metrics_Fields{
		Add: fields,
	}

	metricsPolicy := &api.Policy{
		Key:  name + frontendMetricsPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Frontend{
			Frontend: &api.FrontendPolicySpec{
				Kind: &api.FrontendPolicySpec_Metrics_{
					Metrics: spec,
				},
			},
		},
	}

	logger.Debug("generated metrics policy",
		"policy", policy.Name,
		"agentgateway_policy", metricsPolicy.Name)

	return metricsPolicy, errors.Join(errs...)
}
