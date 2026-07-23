package plugins

import (
	"errors"
	"fmt"
	"net/url"
	"strings"

	jsonpb "google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/types/known/durationpb"
	"google.golang.org/protobuf/types/known/structpb"
	"istio.io/istio/pkg/config"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const (
	aiPolicySuffix                = ":ai"
	backendTlsPolicySuffix        = ":backend-tls"
	backendTunnelPolicySuffix     = ":backend-tunnel"
	backendauthPolicySuffix       = ":backend-auth"
	backendTransformationSuffix   = ":backend-transformation"
	tlsPolicySuffix               = ":tls"
	backendHttpPolicySuffix       = ":backend-http"
	mcpAuthorizationPolicySuffix  = ":mcp-authorization"
	mcpAuthenticationPolicySuffix = ":mcp-authentication"
	mcpGuardrailsPolicySuffix     = ":mcp-guardrails"
	healthPolicySuffix            = ":health"
)

func translateAwsSessionTags(tags []agentgateway.AwsSessionTag) []*api.AwsSessionTag {
	if len(tags) == 0 {
		return nil
	}
	return slices.Map(tags, func(t agentgateway.AwsSessionTag) *api.AwsSessionTag {
		out := &api.AwsSessionTag{Key: t.Key}
		if t.Value != nil {
			out.Value = *t.Value
		}
		if t.Expression != nil {
			out.Expression = string(*t.Expression)
		}
		return out
	})
}

func translateAuthorizationLocation(loc *agentgateway.AuthorizationLocation) *api.AuthorizationLocation {
	if loc == nil {
		return nil
	}
	if loc.Header != nil {
		return &api.AuthorizationLocation{
			Kind: &api.AuthorizationLocation_Header_{
				Header: &api.AuthorizationLocation_Header{
					Name:   string(loc.Header.Name),
					Prefix: loc.Header.Prefix,
				},
			},
		}
	}
	if loc.QueryParameter != nil {
		return &api.AuthorizationLocation{
			Kind: &api.AuthorizationLocation_QueryParameter_{
				QueryParameter: &api.AuthorizationLocation_QueryParameter{
					Name: loc.QueryParameter.Name,
				},
			},
		}
	}
	if loc.Cookie != nil {
		return &api.AuthorizationLocation{
			Kind: &api.AuthorizationLocation_Cookie_{
				Cookie: &api.AuthorizationLocation_Cookie{
					Name: loc.Cookie.Name,
				},
			},
		}
	}
	return nil
}

func translateAuthorizationExtractionLocation(loc *agentgateway.AuthorizationExtractionLocation) *api.AuthorizationLocation {
	if loc == nil {
		return nil
	}
	if translated := translateAuthorizationLocation(&agentgateway.AuthorizationLocation{
		AuthorizationLocationFields: loc.AuthorizationLocationFields,
	}); translated != nil {
		return translated
	}
	if loc.Expression != nil {
		return &api.AuthorizationLocation{
			Kind: &api.AuthorizationLocation_Expression{
				Expression: string(*loc.Expression),
			},
		}
	}
	return nil
}

func validateExtractionAuthorizationLocation(loc *agentgateway.AuthorizationExtractionLocation, context string) error {
	if loc == nil || loc.Expression == nil || isCEL(*loc.Expression) {
		return nil
	}
	return fmt.Errorf("%s expression is not a valid CEL expression: %s", context, *loc.Expression)
}

func TranslateInlineBackendPolicy(
	ctx PolicyCtx,
	namespace string,
	policy *agentgateway.BackendFull,
) ([]*api.BackendPolicySpec, error) {
	dummy := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "inline_policy",
			Namespace: namespace,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{Backend: policy},
	}
	res, err := translateBackendPolicyToAgw(ctx, dummy)
	return slices.MapFilter(res, func(e *api.Policy) **api.BackendPolicySpec {
		return new(e.GetBackend())
	}), err
}

func translateBackendPolicyToAgw(
	ctx PolicyCtx,
	policy *agentgateway.AgentgatewayPolicy,
) ([]*api.Policy, error) {
	backend := policy.Spec.Backend
	if backend == nil {
		return nil, nil
	}
	agwPolicies := make([]*api.Policy, 0)
	var errs []error

	policyName := getBackendPolicyName(policy.Namespace, policy.Name)
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

	if s := backend.HTTP; s != nil {
		appendPolicy("backendHTTP")(translateBackendHTTP(policy), nil)
	}

	if s := backend.Tunnel; s != nil {
		appendPolicy("backendTunnel")(translateBackendTunnel(ctx, policy))
	}

	if s := backend.TLS; s != nil {
		appendPolicy("backendTLS")(translateBackendTLS(ctx, policy))
	}

	if s := backend.TCP; s != nil {
		appendPolicy("backendTCP")(translateBackendTCP(ctx, policy, policyName))
	}

	if s := backend.Health; s != nil {
		appendPolicy("backendHealth")(translateBackendHealthPolicy(policy))
	}

	if s := backend.Transformation; s != nil {
		appendPolicy("backendTransformation")(translateBackendTransformation(policy))
	}

	if s := backend.MCP; s != nil {
		if backend.MCP.Authorization != nil {
			appendPolicy("backendMCPAuthorization")(translateBackendMCPAuthorization(policy))
		}

		if backend.MCP.Authentication != nil {
			appendPolicy("backendMCPAuthentication")(translateBackendMCPAuthentication(ctx, policy))
		}

		if backend.MCP.Guardrails != nil {
			appendPolicy("backendMCPGuardrails")(translateBackendMCPGuardrails(ctx, policy))
		}
	}

	if s := backend.AI; s != nil {
		appendPolicy("backendAI")(translateBackendAI(ctx, policy, policyName))
	}

	if s := backend.Auth; s != nil {
		appendPolicy("backendAuth")(translateBackendAuth(ctx, policy, policyName))
	}

	if s := backend.ExtAuth; s != nil {
		appendPolicy("backendExtAuth")(translateBackendExtAuth(ctx, policy))
	}

	return agwPolicies, errors.Join(errs...)
}

func translateBackendExtAuth(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy) (*api.Policy, error) {
	spec, err := buildExtAuthSpec(ctx, policy.Spec.Backend.ExtAuth, config.NamespacedName(policy))
	return &api.Policy{
		Key:  getBackendPolicyName(policy.Namespace, policy.Name) + extauthPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, config.NamespacedName(policy)),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_ExtAuthz{
					ExtAuthz: spec,
				},
			},
		},
	}, err
}

func translateBackendMCPGuardrails(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy) (*api.Policy, error) {
	var errs []error
	em := policy.Spec.Backend.MCP.Guardrails

	processors := make([]*api.BackendPolicySpec_McpGuardrails_Processor, 0, len(em.Processors))
	for i := range em.Processors {
		p := &em.Processors[i]
		if p.Remote == nil {
			// ExactlyOneOf guards this at admission; skip defensively.
			continue
		}
		be, err := BuildBackendRef(ctx, p.Remote.BackendRef, policy.Namespace)
		if err != nil {
			errs = append(errs, fmt.Errorf("failed to build mcpGuardrails: %v", err))
		}
		metadata := castCELMap(p.Remote.Metadata, func(key string, expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("mcpGuardrails metadata %q is not a valid CEL expression: %s", key, expr))
		})
		methods := make(map[string]api.BackendPolicySpec_McpGuardrails_Phase, len(p.Methods))
		for name, phase := range p.Methods {
			methods[name] = mcpMethodPhase(phase)
		}
		headerName := func(h agentgateway.HeaderName) string { return string(h) }
		processors = append(processors, &api.BackendPolicySpec_McpGuardrails_Processor{
			Kind: &api.BackendPolicySpec_McpGuardrails_Processor_Remote{
				Remote: &api.BackendPolicySpec_McpGuardrails_Remote{
					Target:                   be,
					FailureMode:              mcpGuardrailsFailureMode(p.Remote.FailureMode),
					Metadata:                 metadata,
					AllowedRequestHeaders:    slices.Map(p.Remote.AllowedRequestHeaders, headerName),
					DisallowedRequestHeaders: slices.Map(p.Remote.DisallowedRequestHeaders, headerName),
				},
			},
			Methods: methods,
		})
	}

	spec := &api.BackendPolicySpec_McpGuardrails{Processors: processors}

	return &api.Policy{
		Key:  getBackendPolicyName(policy.Namespace, policy.Name) + mcpGuardrailsPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, config.NamespacedName(policy)),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_McpGuardrails_{McpGuardrails: spec},
			},
		},
	}, errors.Join(errs...)
}

func mcpGuardrailsFailureMode(m agentgateway.FailureMode) api.BackendPolicySpec_McpGuardrails_FailureMode {
	if m == agentgateway.FailOpen {
		return api.BackendPolicySpec_McpGuardrails_ALLOW
	}
	return api.BackendPolicySpec_McpGuardrails_DENY
}

func mcpMethodPhase(p agentgateway.MCPMethodPhase) api.BackendPolicySpec_McpGuardrails_Phase {
	switch p {
	case agentgateway.MCPMethodPhaseRequest:
		return api.BackendPolicySpec_McpGuardrails_REQUEST
	case agentgateway.MCPMethodPhaseResponse:
		return api.BackendPolicySpec_McpGuardrails_RESPONSE
	case agentgateway.MCPMethodPhaseFull:
		return api.BackendPolicySpec_McpGuardrails_FULL
	default:
		return api.BackendPolicySpec_McpGuardrails_OFF
	}
}

func translateBackendHealthPolicy(policy *agentgateway.AgentgatewayPolicy) (*api.Policy, error) {
	var errs []error

	healthPolicy := policy.Spec.Backend.Health

	var evictionProto *api.BackendPolicySpec_Eviction
	if healthPolicy.Eviction != nil {
		var duration *durationpb.Duration
		if healthPolicy.Eviction.Duration != nil {
			duration = durationpb.New(healthPolicy.Eviction.Duration.Duration)
		}

		// Convert 0–100 integer scores into 0.0–1.0 doubles for proto
		var healthThreshold *float64
		if healthPolicy.Eviction.HealthThreshold != nil {
			val := float64(*healthPolicy.Eviction.HealthThreshold) / 100.0
			healthThreshold = &val
		}
		var restoreHealth *float64
		if healthPolicy.Eviction.RestoreHealth != nil {
			val := float64(*healthPolicy.Eviction.RestoreHealth) / 100.0
			restoreHealth = &val
		}

		evictionProto = &api.BackendPolicySpec_Eviction{
			Duration:            duration,
			RestoreHealth:       restoreHealth,
			ConsecutiveFailures: healthPolicy.Eviction.ConsecutiveFailures,
			HealthThreshold:     healthThreshold,
		}
	}

	var unhealthyCondition string
	if healthPolicy.UnhealthyCondition != nil {
		unhealthyCondition = *castCELPtr(healthPolicy.UnhealthyCondition, func(expr agentgateway.CELExpression) {
			errs = append(errs, fmt.Errorf("backend health unhealthyCondition is not a valid CEL expression: %s", expr))
		})
	}

	p := &api.BackendPolicySpec_Health{
		UnhealthyCondition: unhealthyCondition,
		Eviction:           evictionProto,
	}
	evictPolicy := &api.Policy{
		Key:  policy.Namespace + "/" + policy.Name + healthPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_Health_{
					Health: p,
				},
			},
		},
	}

	return evictPolicy, errors.Join(errs...)
}

func translateBackendTCP(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, name string) (*api.Policy, error) {
	// TODO
	return nil, nil
}

func translateBackendTransformation(
	policy *agentgateway.AgentgatewayPolicy,
) (*api.Policy, error) {
	var errs []error
	transformation := policy.Spec.Backend.Transformation

	convertedReq, err := convertTransformSpec(transformation.Request)
	if err != nil {
		errs = append(errs, err)
	}
	convertedResp, err := convertTransformSpec(transformation.Response)
	if err != nil {
		errs = append(errs, err)
	}

	tp := &api.Policy{
		Key:  policy.Namespace + "/" + policy.Name + backendTransformationSuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_Transformation{
					Transformation: &api.TrafficPolicySpec_TransformationPolicy{
						Request:  convertedReq,
						Response: convertedResp,
					},
				},
			},
		},
	}
	logger.Debug("generated backend transformation policy",
		"policy", policy.Name,
		"agentgateway_policy", tp.Name)
	return tp, errors.Join(errs...)
}

func translateBackendTLS(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy) (*api.Policy, error) {
	var errs []error
	tls := policy.Spec.Backend.TLS

	p := &api.BackendPolicySpec_BackendTLS{}

	if len(tls.MtlsCertificateRef) > 0 {
		// Currently we only support one, and enforce this in the API
		mtls := tls.MtlsCertificateRef[0]
		nn := types.NamespacedName{
			Namespace: policy.Namespace,
			Name:      string(mtls.Name),
		}
		data, err := ctx.ResolveCredentialRef(mtls, policy.Namespace)
		if err != nil {
			errs = append(errs, err)
		} else {
			if _, err := ValidateTlsSecretData(nn.Name, nn.Namespace, data); err != nil {
				errs = append(errs, fmt.Errorf("secret %v contains invalid certificate: %v", nn, err))
			}
			p.Cert = data[corev1.TLSCertKey]
			p.Key = data[corev1.TLSPrivateKeyKey]
			if ca, f := data[corev1.ServiceAccountRootCAKey]; f {
				p.Root = ca
			}
		}
	}

	// Build CA bundle from referenced ConfigMaps, if provided
	// If we were using mTLS, we may be overriding the previously set p.Root -- this is intended
	if len(tls.CACertificateRefs) > 0 {
		var sb strings.Builder
		for _, ref := range tls.CACertificateRefs {
			nn := types.NamespacedName{Namespace: policy.Namespace, Name: ref.Name}
			cfgmap := krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterObjectName(nn))
			if cfgmap == nil {
				errs = append(errs, fmt.Errorf("ConfigMap %s not found", nn))
				continue
			}
			pem, err := GetCACertFromConfigMap(ptr.Flatten(cfgmap))
			if err != nil {
				errs = append(errs, fmt.Errorf("error extracting CA cert from ConfigMap %s: %w", nn, err))
				continue
			}
			if sb.Len() > 0 {
				sb.WriteString("\n")
			}
			sb.WriteString(pem)
		}
		// If we have a root set here, set it
		// This may send an empty root, so that we trust nothing rather than system certs.
		p.Root = []byte(sb.String())
	}

	if len(tls.VerifySubjectAltNames) > 0 {
		p.VerifySubjectAltNames = tls.VerifySubjectAltNames
	}
	p.Hostname = tls.Sni

	if tls.InsecureSkipVerify != nil {
		switch *tls.InsecureSkipVerify {
		case agentgateway.InsecureTLSModeAll:
			p.Verification = api.BackendPolicySpec_BackendTLS_INSECURE_ALL
		case agentgateway.InsecureTLSModeHostname:
			p.Verification = api.BackendPolicySpec_BackendTLS_INSECURE_HOST
		}
	}

	if tls.AlpnProtocols != nil {
		p.Alpn = &api.Alpn{Protocols: *tls.AlpnProtocols}
	}
	p.KeyExchangeGroups = convertTLSKeyExchangeGroups(tls.KeyExchangeGroups)

	tlsPolicy := &api.Policy{
		Key:  policy.Namespace + "/" + policy.Name + tlsPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_BackendTls{
					BackendTls: p,
				},
			},
		},
	}

	logger.Debug("generated TLS policy",
		"policy", policy.Name,
		"agentgateway_policy", tlsPolicy.Name)

	return tlsPolicy, errors.Join(errs...)
}

func translateBackendHTTP(policy *agentgateway.AgentgatewayPolicy) *api.Policy {
	http := policy.Spec.Backend.HTTP
	p := &api.BackendPolicySpec_BackendHTTP{}
	if v := http.Version; v != nil {
		switch *v {
		case agentgateway.HTTPVersion1:
			p.Version = api.BackendPolicySpec_BackendHTTP_HTTP1
		case agentgateway.HTTPVersion2:
			p.Version = api.BackendPolicySpec_BackendHTTP_HTTP2
		}
	}
	if rt := http.RequestTimeout; rt != nil {
		p.RequestTimeout = durationpb.New(rt.Duration)
	}
	tp := &api.Policy{
		Key:  policy.Namespace + "/" + policy.Name + backendHttpPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_BackendHttp{
					BackendHttp: p,
				},
			},
		},
	}
	logger.Debug("generated HTTP policy",
		"policy", policy.Name,
		"agentgateway_policy", tp.Name)

	return tp
}

func translateBackendTunnel(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy) (*api.Policy, error) {
	tunnel := policy.Spec.Backend.Tunnel

	proxy, err := BuildBackendRef(ctx, tunnel.BackendRef, policy.Namespace)

	tunnelPolicy := &api.Policy{
		Key:  policy.Namespace + "/" + policy.Name + backendTunnelPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_BackendTunnel_{
					BackendTunnel: &api.BackendPolicySpec_BackendTunnel{
						Proxy: proxy,
					},
				},
			},
		},
	}

	logger.Debug("generated backend tunnel policy",
		"policy", policy.Name,
		"agentgateway_policy", tunnelPolicy.Name)

	return tunnelPolicy, err
}

func translateBackendMCPAuthorization(policy *agentgateway.AgentgatewayPolicy) (*api.Policy, error) {
	backend := policy.Spec.Backend
	if backend == nil || backend.MCP == nil || backend.MCP.Authorization == nil {
		return nil, nil
	}
	auth := backend.MCP.Authorization
	var errs []error
	var allowPolicies, denyPolicies, requirePolicies []string
	policies := castCELSlice(auth.Policy.MatchExpressions, func(expr agentgateway.CELExpression) {
		errs = append(errs, fmt.Errorf("backend MCP authorization matchExpression is not a valid CEL expression: %s", expr))
	})
	if auth.Action == agentgateway.AuthorizationPolicyActionDeny {
		denyPolicies = append(denyPolicies, policies...)
	} else if auth.Action == agentgateway.AuthorizationPolicyActionRequire {
		requirePolicies = append(requirePolicies, policies...)
	} else {
		allowPolicies = append(allowPolicies, policies...)
	}

	mcpPolicy := &api.Policy{
		Key:  policy.Namespace + "/" + policy.Name + mcpAuthorizationPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_McpAuthorization_{
					McpAuthorization: &api.BackendPolicySpec_McpAuthorization{
						Allow:   allowPolicies,
						Deny:    denyPolicies,
						Require: requirePolicies,
					},
				},
			},
		},
	}

	logger.Debug("generated MCPBackend policy",
		"policy", policy.Name,
		"agentgateway_policy", mcpPolicy.Name)

	return mcpPolicy, errors.Join(errs...)
}

func translateBackendMCPAuthentication(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy) (*api.Policy, error) {
	authnPolicy := policy.Spec.Backend.MCP.Authentication

	mcpAuthn, err := translateMCPAuthenticationSpec(ctx, types.NamespacedName{
		Namespace: policy.Namespace,
		Name:      policy.Name,
	}, authnPolicy)
	mcpAuthnPolicy := &api.Policy{
		Key:  policy.Namespace + "/" + policy.Name + mcpAuthenticationPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_McpAuthentication_{
					McpAuthentication: mcpAuthn,
				},
			},
		},
	}

	logger.Debug("generated MCP authentication policy",
		"policy", policy.Name,
		"agentgateway_policy", mcpAuthnPolicy.Name)

	return mcpAuthnPolicy, err
}

func translateMCPAuthenticationSpec(
	ctx PolicyCtx,
	policy types.NamespacedName,
	authnPolicy *agentgateway.MCPAuthentication,
) (*api.BackendPolicySpec_McpAuthentication, error) {
	idp := translateMcpIDP(authnPolicy.McpIDP)

	// default mode is Strict
	mode := api.BackendPolicySpec_McpAuthentication_STRICT
	if authnPolicy.Mode == agentgateway.JWTAuthenticationModeStrict {
		mode = api.BackendPolicySpec_McpAuthentication_STRICT
	} else if authnPolicy.Mode == agentgateway.JWTAuthenticationModePermissive {
		mode = api.BackendPolicySpec_McpAuthentication_PERMISSIVE
	} else if authnPolicy.Mode == agentgateway.JWTAuthenticationModeOptional {
		mode = api.BackendPolicySpec_McpAuthentication_OPTIONAL
	}

	var errs []error
	translatedInlineJwks, err := resolveJWKSInlineForOwner(
		ctx,
		jwks.PolicyBackendMCPAuthenticationLookupOwner(policy.Namespace, policy.Name, authnPolicy.JWKS),
	)
	if err != nil {
		logger.Error("failed resolving jwks", "error", err)
		errs = append(errs, err)
	}

	extraResourceMetadata, metadataErr := translateMCPResourceMetadata(authnPolicy.ResourceMetadata)
	if metadataErr != nil {
		errs = append(errs, metadataErr)
	}

	mcpAuthn := &api.BackendPolicySpec_McpAuthentication{
		Issuer:    authnPolicy.Issuer,
		Audiences: authnPolicy.Audiences,
		Provider:  idp,
		ResourceMetadata: &api.BackendPolicySpec_McpAuthentication_ResourceMetadata{
			Extra: extraResourceMetadata,
		},
		JwksInline: translatedInlineJwks,
		Mode:       mode,
		ClientId:   authnPolicy.ClientID,
	}

	if authnPolicy.ClientSecretRef != nil {
		data, key, err := ctx.ResolveCredentialKeyRef(*authnPolicy.ClientSecretRef, policy.Namespace, wellknown.ClientSecret)
		if err != nil {
			errs = append(errs, err)
		} else if value, exists := kubeutils.GetSecretDataValue(data, key); !exists || value == "" {
			errs = append(errs, fmt.Errorf("secret %s/%s missing %s value", policy.Namespace, authnPolicy.ClientSecretRef.Name, key))
		} else {
			mcpAuthn.ClientSecret = &value
		}
	}

	return mcpAuthn, errors.Join(errs...)
}

func translateJWTMCPConfig(mcp *agentgateway.JWTMCPConfig) (*api.TrafficPolicySpec_JWT_MCP, error) {
	extraResourceMetadata, err := translateMCPResourceMetadata(mcp.ResourceMetadata)
	if err != nil {
		return nil, err
	}

	return &api.TrafficPolicySpec_JWT_MCP{
		Provider: translateMcpIDP(mcp.Provider),
		ResourceMetadata: &api.BackendPolicySpec_McpAuthentication_ResourceMetadata{
			Extra: extraResourceMetadata,
		},
		ClientId: mcp.ClientID,
	}, nil
}

func translateMcpIDP(provider *agentgateway.McpIDP) api.BackendPolicySpec_McpAuthentication_McpIDP {
	if provider == nil {
		return api.BackendPolicySpec_McpAuthentication_UNSPECIFIED
	}
	if *provider == agentgateway.Keycloak {
		return api.BackendPolicySpec_McpAuthentication_KEYCLOAK
	}
	if *provider == agentgateway.Auth0 {
		return api.BackendPolicySpec_McpAuthentication_AUTH0
	}
	if *provider == agentgateway.Okta {
		return api.BackendPolicySpec_McpAuthentication_OKTA
	}
	if *provider == agentgateway.Descope {
		return api.BackendPolicySpec_McpAuthentication_DESCOPE
	}
	if *provider == agentgateway.Authentik {
		return api.BackendPolicySpec_McpAuthentication_AUTHENTIK
	}
	if *provider == agentgateway.Entra {
		return api.BackendPolicySpec_McpAuthentication_ENTRA
	}
	return api.BackendPolicySpec_McpAuthentication_UNSPECIFIED
}

func translateMCPResourceMetadata(resourceMetadata map[string]apiextensionsv1.JSON) (map[string]*structpb.Value, error) {
	var errs []error
	var extraResourceMetadata map[string]*structpb.Value
	for k, v := range resourceMetadata {
		if extraResourceMetadata == nil {
			extraResourceMetadata = make(map[string]*structpb.Value)
		}

		proto := &structpb.Value{}
		err := jsonpb.Unmarshal(v.Raw, proto)
		if err != nil {
			logger.Error("error converting resource metadata", "key", k, "error", err)
			errs = append(errs, err)
			continue
		}

		extraResourceMetadata[k] = proto
	}
	return extraResourceMetadata, errors.Join(errs...)
}

// translateBackendAI processes AI configuration and creates corresponding Agw policies
func translateBackendAI(ctx PolicyCtx, agwPolicy *agentgateway.AgentgatewayPolicy, name string) (*api.Policy, error) {
	var errs []error
	aiSpec := agwPolicy.Spec.Backend.AI

	translatedAIPolicy := &api.BackendPolicySpec_Ai{}
	if aiSpec.PromptEnrichment != nil {
		translatedAIPolicy.Prompts = processPromptEnrichment(aiSpec.PromptEnrichment)
	}

	for _, def := range aiSpec.Defaults {
		val, err := toJSONValue(def.Value)
		if err != nil {
			logger.Error("error parsing field value", "field", def.Field, "error", err)
			errs = append(errs, err)
			continue
		}
		if translatedAIPolicy.Defaults == nil {
			translatedAIPolicy.Defaults = make(map[string]string)
		}
		translatedAIPolicy.Defaults[def.Field] = val
	}

	for _, def := range aiSpec.Overrides {
		val, err := toJSONValue(def.Value)
		if err != nil {
			logger.Error("error parsing field value", "field", def.Field, "error", err)
			errs = append(errs, err)
			continue
		}
		if translatedAIPolicy.Overrides == nil {
			translatedAIPolicy.Overrides = make(map[string]string)
		}
		translatedAIPolicy.Overrides[def.Field] = val
	}
	for _, xfm := range aiSpec.Transformations {
		if translatedAIPolicy.Transformations == nil {
			translatedAIPolicy.Transformations = make(map[string]string)
		}

		if !isCEL(xfm.Expression) {
			errs = append(errs, fmt.Errorf("transformation %q is not a valid CEL expression: %v", xfm.Field, xfm.Expression))
		}

		// Still set it so it wipes out the value on error, mirroring the header value.
		translatedAIPolicy.Transformations[xfm.Field] = string(xfm.Expression)
	}

	if aiSpec.PromptGuard != nil {
		if translatedAIPolicy.PromptGuard == nil {
			translatedAIPolicy.PromptGuard = &api.BackendPolicySpec_Ai_PromptGuard{}
		}
		if aiSpec.PromptGuard.Streaming == agentgateway.PromptGuardStreamingModeEnabled {
			translatedAIPolicy.PromptGuard.Streaming = api.BackendPolicySpec_Ai_PromptGuard_ENABLED
		}
		if aiSpec.PromptGuard.Request != nil {
			r, err := processRequestGuard(ctx, agwPolicy.Namespace, aiSpec.PromptGuard.Request)
			if err != nil {
				logger.Error("error parsing request prompt guard", "error", err)
				errs = append(errs, err)
			} else {
				translatedAIPolicy.PromptGuard.Request = r
			}
		}

		if aiSpec.PromptGuard.Response != nil {
			r, err := processResponseGuard(ctx, agwPolicy.Namespace, aiSpec.PromptGuard.Response)
			if err != nil {
				logger.Error("error parsing response prompt guard", "error", err)
				errs = append(errs, err)
			} else {
				translatedAIPolicy.PromptGuard.Response = r
			}
		}
	}

	if aiSpec.ModelAliases != nil {
		translatedAIPolicy.ModelAliases = aiSpec.ModelAliases
	}

	if aiSpec.PromptCaching != nil {
		translatedAIPolicy.PromptCaching = &api.BackendPolicySpec_Ai_PromptCaching{
			CacheSystem:   aiSpec.PromptCaching.CacheSystem,
			CacheMessages: aiSpec.PromptCaching.CacheMessages,
			CacheTools:    aiSpec.PromptCaching.CacheTools,
		}
		translatedAIPolicy.PromptCaching.MinTokens = new(uint32(aiSpec.PromptCaching.MinTokens)) //nolint:gosec // G115: MinTokens is validated by kubebuilder to be >= 0
		if aiSpec.PromptCaching.CacheMessageOffset > 0 {
			translatedAIPolicy.PromptCaching.CacheMessageOffset = new(uint32(aiSpec.PromptCaching.CacheMessageOffset)) //nolint:gosec // G115: CacheMessageOffset is validated by kubebuilder to be >= 0
		}
	}

	if aiSpec.Routes != nil {
		r := make(map[string]api.BackendPolicySpec_Ai_RouteType)
		for path, routeType := range aiSpec.Routes {
			r[path] = translateRouteType(routeType)
		}
		translatedAIPolicy.Routes = r
	}

	aiPolicy := &api.Policy{
		Key:  name + aiPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, agwPolicy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_Ai_{
					Ai: translatedAIPolicy,
				},
			},
		},
	}

	logger.Debug("generated AI policy",
		"policy", agwPolicy.Name,
		"agentgateway_policy", aiPolicy.Name)

	return aiPolicy, errors.Join(errs...)
}

func translateBackendAuth(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, name string) (*api.Policy, error) {
	var errs []error
	auth := policy.Spec.Backend.Auth

	var translatedAuth *api.BackendAuthPolicy
	var kindErrs []error
	if auth.InlineKey != nil && *auth.InlineKey != "" {
		translatedAuth = &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Key{
				Key: &api.Key{
					Secret:                *auth.InlineKey,
					AuthorizationLocation: translateAuthorizationLocation(auth.Location),
				},
			},
		}
	} else if auth.SecretRef != nil {
		// Resolve secret and extract the authorization value
		data, key, err := ctx.ResolveCredentialKeyRef(*auth.SecretRef, policy.Namespace, wellknown.Authorization)
		if err != nil {
			errs = append(errs, err)
			translatedAuth = &api.BackendAuthPolicy{
				Kind: &api.BackendAuthPolicy_Key{
					Key: &api.Key{
						AuthorizationLocation: translateAuthorizationLocation(auth.Location),
					},
				},
			}
		} else {
			var authKey string
			var ok bool
			if key == wellknown.Authorization {
				authKey, ok = kubeutils.GetSecretDataAuth(data)
			} else {
				authKey, ok = kubeutils.GetSecretDataValue(data, key)
			}
			if ok && authKey != "" {
				translatedAuth = &api.BackendAuthPolicy{
					Kind: &api.BackendAuthPolicy_Key{
						Key: &api.Key{
							Secret:                authKey,
							AuthorizationLocation: translateAuthorizationLocation(auth.Location),
						},
					},
				}
			} else {
				errs = append(errs, fmt.Errorf("secret %s/%s missing %s value", policy.Namespace, auth.SecretRef.Name, key))
				translatedAuth = &api.BackendAuthPolicy{
					Kind: &api.BackendAuthPolicy_Key{
						Key: &api.Key{
							AuthorizationLocation: translateAuthorizationLocation(auth.Location),
						},
					},
				}
			}
		}
	} else if auth.AWS != nil {
		awsAuth, err := buildAwsAuthPolicy(ctx, auth.AWS, policy.Namespace)
		translatedAuth = awsAuth
		if err != nil {
			errs = append(errs, err)
		}
	} else if auth.Azure != nil {
		azureAuth, err := buildAzureAuthPolicy(ctx, auth.Azure, policy.Namespace)
		translatedAuth = azureAuth
		if err != nil {
			errs = append(errs, err)
		}
	} else if auth.GCP != nil {
		gcpAuth, err := buildGcpAuthPolicy(ctx, auth.GCP, policy.Namespace)
		translatedAuth = gcpAuth
		if err != nil {
			errs = append(errs, err)
		}
	} else if auth.OAuthTokenExchange != nil {
		oauthAuth, err := buildOAuthTokenExchangePolicy(ctx, auth.OAuthTokenExchange, policy.Namespace)
		translatedAuth = oauthAuth
		if err != nil {
			errs = append(errs, err)
		}
	} else if auth.CrossAppAccess != nil {
		crossAppAccessAuth, err := buildCrossAppAccessPolicy(ctx, auth.CrossAppAccess, policy.Namespace)
		translatedAuth = crossAppAccessAuth
		if err != nil {
			errs = append(errs, err)
		}
	} else if auth.Passthrough != nil {
		translatedAuth = &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Passthrough{
				Passthrough: &api.Passthrough{
					AuthorizationLocation: translateAuthorizationLocation(auth.Location),
				},
			},
		}
	}

	translatedCredentials, credErrs := translateBackendAuthCredentials(ctx, auth.Credentials, policy.Namespace)
	errs = append(errs, credErrs...)
	if len(translatedCredentials) > 0 {
		if translatedAuth == nil {
			translatedAuth = &api.BackendAuthPolicy{}
		}
		translatedAuth.Credentials = translatedCredentials
	}

	if translatedAuth == nil {
		return nil, errors.Join(append(errs, kindErrs...)...)
	}

	authPolicy := &api.Policy{
		Key:  name + backendauthPolicySuffix,
		Name: TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_Auth{
					Auth: translatedAuth,
				},
			},
		},
	}
	logger.Debug("generated backend auth policy",
		"policy", policy.Name,
		"agentgateway_policy", authPolicy.Name)

	return authPolicy, errors.Join(append(errs, kindErrs...)...)
}

var oauthReservedAdditionalParams = []string{
	"grant_type",
	"subject_token",
	"subject_token_type",
	"actor_token",
	"actor_token_type",
	"assertion",
	"audience",
	"resource",
	"scope",
	"requested_token_type",
	"client_id",
	"client_secret",
	"client_assertion",
	"client_assertion_type",
}

func buildOAuthTokenExchangePolicy(ctx PolicyCtx, auth *agentgateway.OAuthTokenExchange, namespace string) (*api.BackendAuthPolicy, error) {
	oauth, err := BuildOAuthTokenExchange(ctx, auth, namespace, nil)
	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_OauthTokenExchange{
			OauthTokenExchange: oauth,
		},
	}, err
}

func BuildCrossAppAccess(ctx PolicyCtx, auth *agentgateway.CrossAppAccessAuth, namespace string) (*api.CrossAppAccessAuth, error) {
	if auth == nil {
		return nil, errors.New("crossAppAccess must not be nil")
	}

	var errs []error
	if auth.Audience == "" {
		errs = append(errs, errors.New("crossAppAccess audience must not be empty"))
	}

	identityProvider, err := buildCrossAppAccessEndpoint(ctx, &auth.IdentityProvider, namespace, "crossAppAccess.identityProvider")
	if err != nil {
		errs = append(errs, err)
	}
	resourceAuthorizationServer, err := buildCrossAppAccessEndpoint(ctx, &auth.ResourceAuthorizationServer, namespace, "crossAppAccess.resourceAuthorizationServer")
	if err != nil {
		errs = append(errs, err)
	}

	if auth.SubjectToken != nil {
		if err := validateExtractionAuthorizationLocation(auth.SubjectToken.Source, "crossAppAccess subjectToken source"); err != nil {
			errs = append(errs, err)
		}
	}

	return &api.CrossAppAccessAuth{
		IdentityProvider:            identityProvider,
		ResourceAuthorizationServer: resourceAuthorizationServer,
		Audience:                    auth.Audience,
		Resources:                   auth.Resources,
		Scopes:                      auth.Scopes,
		SubjectToken:                translateCrossAppAccessSubjectToken(auth.SubjectToken),
		Cache:                       translateOAuthTokenCache(auth.Cache),
	}, errors.Join(errs...)
}

func translateCrossAppAccessSubjectToken(spec *agentgateway.CrossAppAccessSubjectToken) *api.CrossAppAccessAuth_SubjectToken {
	if spec == nil {
		return nil
	}
	return &api.CrossAppAccessAuth_SubjectToken{
		Source: translateAuthorizationExtractionLocation(spec.Source),
	}
}

func buildCrossAppAccessPolicy(ctx PolicyCtx, auth *agentgateway.CrossAppAccessAuth, namespace string) (*api.BackendAuthPolicy, error) {
	crossAppAccess, err := BuildCrossAppAccess(ctx, auth, namespace)
	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_CrossAppAccess{
			CrossAppAccess: crossAppAccess,
		},
	}, err
}

func buildCrossAppAccessEndpoint(ctx PolicyCtx, endpoint *agentgateway.CrossAppAccessEndpoint, namespace, field string) (*api.CrossAppAccessAuth_Endpoint, error) {
	var errs []error

	tokenEndpoint, err := BuildBackendRef(ctx, endpoint.BackendRef, namespace)
	if err != nil {
		errs = append(errs, err)
	}
	clientAuth, err := buildOAuthClientAuth(ctx, &endpoint.ClientAuth, namespace)
	if err != nil {
		errs = append(errs, err)
	}

	if endpoint.Path != nil && !strings.HasPrefix(*endpoint.Path, "/") {
		errs = append(errs, fmt.Errorf("%s.path %q must start with /", field, *endpoint.Path))
	}

	return &api.CrossAppAccessAuth_Endpoint{
		TokenEndpoint:     tokenEndpoint,
		TokenEndpointPath: endpoint.Path,
		ClientAuth:        clientAuth,
	}, errors.Join(errs...)
}

// BuildOAuthTokenExchange lowers an OAuth token exchange policy into its xDS representation.
func BuildOAuthTokenExchange(ctx PolicyCtx, auth *agentgateway.OAuthTokenExchange, namespace string, tokenEndpoint *api.BackendReference) (*api.OAuthTokenExchange, error) {
	if auth == nil {
		return nil, errors.New("oauthTokenExchange must not be nil")
	}

	var errs []error

	if tokenEndpoint == nil {
		var err error
		tokenEndpoint, err = BuildBackendRef(ctx, auth.BackendRef, namespace)
		if err != nil {
			errs = append(errs, err)
		}
	}

	additionalParams := castCELMap(auth.AdditionalParams, func(key string, expr agentgateway.CELExpression) {
		errs = append(errs, fmt.Errorf("oauth additionalParams %q is not a valid CEL expression: %s", key, expr))
	})
	for key := range auth.AdditionalParams {
		if isOAuthReservedAdditionalParam(key) {
			errs = append(errs, fmt.Errorf("oauth additionalParams %q overrides a reserved OAuth parameter", key))
		}
	}

	clientAuth, err := buildOAuthClientAuth(ctx, auth.ClientAuth, namespace)
	if err != nil {
		errs = append(errs, err)
	}

	if auth.SubjectToken != nil {
		if err := validateExtractionAuthorizationLocation(auth.SubjectToken.Source, "oauth subjectToken source"); err != nil {
			errs = append(errs, err)
		}
		if auth.SubjectToken.TokenType != nil {
			errs = append(errs, validateOAuthTokenType(*auth.SubjectToken.TokenType, "oauth subjectToken tokenType"))
		}
	}
	if auth.ActorToken != nil {
		if err := validateExtractionAuthorizationLocation(&auth.ActorToken.Source, "oauth actorToken source"); err != nil {
			errs = append(errs, err)
		}
		if auth.ActorToken.TokenType != nil {
			errs = append(errs, validateOAuthTokenType(*auth.ActorToken.TokenType, "oauth actorToken tokenType"))
		}
	}

	oauth := &api.OAuthTokenExchange{
		TokenEndpoint:         tokenEndpoint,
		TokenEndpointPath:     auth.Path,
		GrantType:             translateOAuthGrantType(auth.GrantType),
		SubjectToken:          translateOAuthTokenSpec(auth.SubjectToken),
		ActorToken:            translateOAuthActorToken(auth.ActorToken),
		Audiences:             auth.Audiences,
		Scopes:                auth.Scopes,
		Resources:             auth.Resources,
		RequestedTokenType:    translateOAuthTokenTypePtr(auth.RequestedTokenType),
		AdditionalParams:      additionalParams,
		ClientAuth:            clientAuth,
		AuthorizationLocation: translateAuthorizationLocation(auth.Location),
		Cache:                 translateOAuthTokenCache(auth.Cache),
	}
	if auth.Path != nil && !strings.HasPrefix(*auth.Path, "/") {
		errs = append(errs, fmt.Errorf("oauthTokenExchange.path %q must start with /", *auth.Path))
	}
	if oauth.GrantType == api.OAuthTokenExchange_JWT_BEARER {
		if oauth.ActorToken != nil {
			errs = append(errs, errors.New("oauth actorToken is only valid with TokenExchange grantType"))
		}
		if oauth.RequestedTokenType != nil {
			errs = append(errs, errors.New("oauth requestedTokenType is only valid with TokenExchange grantType"))
		}
	}
	if auth.RequestedTokenType != nil && *auth.RequestedTokenType == agentgateway.OAuthTokenTypeIDJAG {
		errs = append(errs, errors.New("oauth requestedTokenType IdJag is only supported by crossAppAccess"))
	}
	if auth.ActorToken != nil && ptr.OrDefault(auth.ActorToken.MayAct, "") == agentgateway.OAuthMayActValidationModeRequired && ptr.OrDefault(auth.ActorToken.TokenType, "") != agentgateway.OAuthTokenTypeJWT {
		errs = append(errs, errors.New("oauth actorToken mayAct Required requires tokenType Jwt"))
	}

	return oauth, errors.Join(errs...)
}

func translateOAuthGrantType(grantType *agentgateway.OAuthGrantType) api.OAuthTokenExchange_GrantType {
	if grantType == nil {
		return api.OAuthTokenExchange_UNSPECIFIED
	}
	switch *grantType {
	case agentgateway.OAuthGrantTypeJwtBearer:
		return api.OAuthTokenExchange_JWT_BEARER
	case agentgateway.OAuthGrantTypeTokenExchange:
		return api.OAuthTokenExchange_TOKEN_EXCHANGE
	default:
		return api.OAuthTokenExchange_UNSPECIFIED
	}
}

func translateOAuthTokenSpec(spec *agentgateway.OAuthTokenSpec) *api.OAuthTokenExchange_TokenSpec {
	if spec == nil {
		return nil
	}
	res := &api.OAuthTokenExchange_TokenSpec{
		Source: translateAuthorizationExtractionLocation(spec.Source),
	}
	if spec.TokenType != nil {
		res.TokenType = translateOAuthTokenType(*spec.TokenType)
	}
	return res
}

func translateOAuthActorToken(actor *agentgateway.OAuthActorToken) *api.OAuthTokenExchange_ActorToken {
	if actor == nil {
		return nil
	}
	res := &api.OAuthTokenExchange_ActorToken{
		Source:        translateAuthorizationExtractionLocation(&actor.Source),
		EnforceMayAct: ptr.OrDefault(actor.MayAct, "") == agentgateway.OAuthMayActValidationModeRequired,
	}
	if actor.TokenType != nil {
		res.TokenType = translateOAuthTokenType(*actor.TokenType)
	}
	return res
}

func buildOAuthClientAuth(ctx PolicyCtx, auth *agentgateway.OAuthClientAuth, namespace string) (*api.OAuthClientAuth, error) {
	if auth == nil {
		return nil, nil
	}

	var errs []error
	res := &api.OAuthClientAuth{
		ClientId: auth.ClientID,
		Method:   translateOAuthClientAuthMethod(auth.Method),
	}
	if auth.ClientID == "" {
		errs = append(errs, errors.New("oauth clientAuth clientId must not be empty"))
	}

	if auth.SecretRef != nil && auth.PrivateKeyJWT != nil {
		errs = append(errs, errors.New("oauth clientAuth secretRef is not valid with privateKeyJwt"))
	}

	if auth.PrivateKeyJWT != nil {
		if auth.Method == nil || *auth.Method != agentgateway.OAuthClientAuthMethodPrivateKeyJWT {
			errs = append(errs, errors.New("oauth clientAuth privateKeyJwt requires method PrivateKeyJwt"))
		}
		privateKeyJWT, err := buildOAuthPrivateKeyJWT(ctx, auth.PrivateKeyJWT, namespace)
		res.PrivateKeyJwt = privateKeyJWT
		if err != nil {
			errs = append(errs, err)
		}
	} else if auth.Method != nil && *auth.Method == agentgateway.OAuthClientAuthMethodPrivateKeyJWT {
		errs = append(errs, errors.New("oauth clientAuth method PrivateKeyJwt requires privateKeyJwt settings"))
	}

	if auth.SecretRef != nil && auth.PrivateKeyJWT == nil {
		data, key, err := ctx.ResolveCredentialKeyRef(*auth.SecretRef, namespace, wellknown.ClientSecret)
		if err != nil {
			clientSecret := ""
			res.ClientSecret = &clientSecret
			errs = append(errs, err)
		} else if value, exists := kubeutils.GetSecretDataValue(data, key); !exists || value == "" {
			clientSecret := ""
			res.ClientSecret = &clientSecret
			errs = append(errs, fmt.Errorf("secret %s/%s missing %s value", namespace, auth.SecretRef.Name, key))
		} else {
			res.ClientSecret = &value
		}
	} else if auth.PrivateKeyJWT == nil && (auth.Method == nil || *auth.Method != agentgateway.OAuthClientAuthMethodClientSecretPost) {
		errs = append(errs, errors.New("oauth clientAuth without secretRef requires method ClientSecretPost or PrivateKeyJwt"))
	}

	return res, errors.Join(errs...)
}

func buildOAuthPrivateKeyJWT(ctx PolicyCtx, auth *agentgateway.OAuthPrivateKeyJWT, namespace string) (*api.OAuthClientAuth_PrivateKeyJwt, error) {
	if auth == nil {
		return nil, nil
	}

	var errs []error
	res := &api.OAuthClientAuth_PrivateKeyJwt{
		Alg:               translateOAuthPrivateKeyJWTSigningAlg(auth.Alg),
		Kid:               auth.KeyID,
		AssertionAudience: auth.AssertionAudience,
	}

	if auth.AssertionAudience == "" {
		errs = append(errs, errors.New("oauth clientAuth privateKeyJwt assertionAudience must not be empty"))
	}

	data, key, err := ctx.ResolveCredentialKeyRef(auth.SigningKeyRef, namespace, wellknown.SigningKey)
	if err != nil {
		errs = append(errs, err)
	} else if value, exists := kubeutils.GetSecretDataValue(data, key); !exists || value == "" {
		errs = append(errs, fmt.Errorf("secret %s/%s missing %s value", namespace, auth.SigningKeyRef.Name, key))
	} else {
		res.SigningKey = value
	}

	return res, errors.Join(errs...)
}

func translateOAuthTokenTypePtr(tokenType *agentgateway.OAuthTokenType) *string {
	if tokenType == nil {
		return nil
	}
	return new(translateOAuthTokenType(*tokenType))
}

func validateOAuthTokenType(tokenType agentgateway.OAuthTokenType, field string) error {
	switch tokenType {
	case agentgateway.OAuthTokenTypeAccessToken,
		agentgateway.OAuthTokenTypeJWT,
		agentgateway.OAuthTokenTypeIDToken,
		agentgateway.OAuthTokenTypeIDJAG:
		return nil
	}
	parsed, err := url.Parse(string(tokenType))
	if err != nil || !parsed.IsAbs() || parsed.Fragment != "" {
		return fmt.Errorf("%s %q must be a built-in token type or an absolute URI without a fragment", field, tokenType)
	}
	return nil
}

func translateOAuthTokenType(tokenType agentgateway.OAuthTokenType) string {
	switch tokenType {
	case agentgateway.OAuthTokenTypeAccessToken:
		return "urn:ietf:params:oauth:token-type:access_token"
	case agentgateway.OAuthTokenTypeJWT:
		return "urn:ietf:params:oauth:token-type:jwt"
	case agentgateway.OAuthTokenTypeIDToken:
		return "urn:ietf:params:oauth:token-type:id_token"
	case agentgateway.OAuthTokenTypeIDJAG:
		return "urn:ietf:params:oauth:token-type:id-jag"
	default:
		return string(tokenType)
	}
}

func translateOAuthClientAuthMethod(method *agentgateway.OAuthClientAuthMethod) api.OAuthClientAuth_Method {
	if method == nil {
		return api.OAuthClientAuth_UNSPECIFIED
	}
	switch *method {
	case agentgateway.OAuthClientAuthMethodClientSecretPost:
		return api.OAuthClientAuth_CLIENT_SECRET_POST
	case agentgateway.OAuthClientAuthMethodClientSecretBasic:
		return api.OAuthClientAuth_CLIENT_SECRET_BASIC
	case agentgateway.OAuthClientAuthMethodPrivateKeyJWT:
		return api.OAuthClientAuth_PRIVATE_KEY_JWT
	default:
		return api.OAuthClientAuth_UNSPECIFIED
	}
}

func translateOAuthPrivateKeyJWTSigningAlg(alg *agentgateway.OAuthPrivateKeyJWTSigningAlgorithm) api.OAuthClientAuth_PrivateKeyJwt_SigningAlg {
	if alg == nil {
		return api.OAuthClientAuth_PrivateKeyJwt_SIGNING_ALG_UNSPECIFIED
	}
	switch *alg {
	case agentgateway.OAuthPrivateKeyJWTSigningAlgorithmRS256:
		return api.OAuthClientAuth_PrivateKeyJwt_RS256
	case agentgateway.OAuthPrivateKeyJWTSigningAlgorithmRS384:
		return api.OAuthClientAuth_PrivateKeyJwt_RS384
	case agentgateway.OAuthPrivateKeyJWTSigningAlgorithmRS512:
		return api.OAuthClientAuth_PrivateKeyJwt_RS512
	case agentgateway.OAuthPrivateKeyJWTSigningAlgorithmES256:
		return api.OAuthClientAuth_PrivateKeyJwt_ES256
	case agentgateway.OAuthPrivateKeyJWTSigningAlgorithmES384:
		return api.OAuthClientAuth_PrivateKeyJwt_ES384
	default:
		return api.OAuthClientAuth_PrivateKeyJwt_SIGNING_ALG_UNSPECIFIED
	}
}

func translateOAuthTokenCache(cache *agentgateway.OAuthTokenCache) *api.OAuthTokenExchange_TokenCache {
	if cache == nil {
		return nil
	}
	res := &api.OAuthTokenExchange_TokenCache{}
	if cache.InMemory != nil {
		res.InMemory = &api.OAuthTokenExchange_TokenCache_InMemory{
			MaxEntries: cache.InMemory.MaxEntries,
		}
		if cache.InMemory.DefaultTTL != nil {
			res.InMemory.DefaultTtl = durationpb.New(cache.InMemory.DefaultTTL.Duration)
		}
	}
	return res
}

func isOAuthReservedAdditionalParam(key string) bool {
	for _, reserved := range oauthReservedAdditionalParams {
		if strings.EqualFold(reserved, key) {
			return true
		}
	}
	return false
}

// translateBackendAuthCredentials resolves BackendAuth credential entries.
func translateBackendAuthCredentials(ctx PolicyCtx, creds []agentgateway.BackendAuthCredential, namespace string) ([]*api.BackendAuthCredential, []error) {
	if len(creds) == 0 {
		return nil, nil
	}
	var errs []error
	translated := make([]*api.BackendAuthCredential, 0, len(creds))
	for _, c := range creds {
		locName := credentialLocationName(c.Location)
		var value string
		data, secretKey, err := ctx.ResolveCredentialKeyRef(c.SecretRef, namespace, wellknown.Authorization)
		if err != nil {
			errs = append(errs, fmt.Errorf("backendAuth credential %q: %w", locName, err))
		} else if resolvedValue, ok := kubeutils.GetSecretDataValue(data, secretKey); !ok {
			errs = append(errs, fmt.Errorf("backendAuth credential %q: secret %s/%s missing key %q",
				locName, namespace, c.SecretRef.Name, secretKey))
		} else {
			value = resolvedValue
		}
		translated = append(translated, &api.BackendAuthCredential{
			Location: translateAuthorizationLocation(&c.Location),
			Value:    value,
		})
	}
	return translated, errs
}

// credentialLocationName returns the name field of whichever location variant is set.
// AuthorizationLocation is guaranteed by CEL to have exactly one of header/queryParameter/cookie set.
func credentialLocationName(loc agentgateway.AuthorizationLocation) string {
	if loc.Header != nil {
		return string(loc.Header.Name)
	}
	if loc.QueryParameter != nil {
		return loc.QueryParameter.Name
	}
	if loc.Cookie != nil {
		return loc.Cookie.Name
	}
	return ""
}

// translateRouteType converts RouteType to agentgateway proto RouteType
func translateRouteType(rt agentgateway.RouteType) api.BackendPolicySpec_Ai_RouteType {
	switch rt {
	case agentgateway.RouteTypeCompletions:
		return api.BackendPolicySpec_Ai_COMPLETIONS
	case agentgateway.RouteTypeMessages:
		return api.BackendPolicySpec_Ai_MESSAGES
	case agentgateway.RouteTypeModels:
		return api.BackendPolicySpec_Ai_MODELS
	case agentgateway.RouteTypePassthrough:
		return api.BackendPolicySpec_Ai_PASSTHROUGH
	case agentgateway.RouteTypeDetect:
		return api.BackendPolicySpec_Ai_DETECT
	case agentgateway.RouteTypeResponses:
		return api.BackendPolicySpec_Ai_RESPONSES
	case agentgateway.RouteTypeAnthropicTokenCount:
		return api.BackendPolicySpec_Ai_ANTHROPIC_TOKEN_COUNT
	case agentgateway.RouteTypeEmbeddings:
		return api.BackendPolicySpec_Ai_EMBEDDINGS
	case agentgateway.RouteTypeRealtime:
		return api.BackendPolicySpec_Ai_REALTIME
	case agentgateway.RouteTypeRerank:
		return api.BackendPolicySpec_Ai_RERANK
	default:
		// Default to completions if unknown type
		return api.BackendPolicySpec_Ai_COMPLETIONS
	}
}

func buildAwsAuthPolicy(ctx PolicyCtx, auth *agentgateway.AwsAuth, namespace string) (*api.BackendAuthPolicy, error) {
	var errs []error

	var accessKeyId, secretAccessKey string
	var sessionToken *string
	var serviceName string
	if auth.ServiceName != nil {
		serviceName = string(*auth.ServiceName)
	}
	var region string
	if auth.Region != nil {
		region = string(*auth.Region)
	}
	var assumeRole *api.AwsAssumeRole
	if auth.AssumeRole != nil {
		assumeRole = &api.AwsAssumeRole{
			RoleArn: auth.AssumeRole.RoleArn,
			Tags:    translateAwsSessionTags(auth.AssumeRole.Tags),
		}
		if auth.AssumeRole.SessionName != nil {
			assumeRole.SessionName = *auth.AssumeRole.SessionName
		}
		if auth.AssumeRole.SessionNameExpression != nil {
			assumeRole.SessionNameExpression = string(*auth.AssumeRole.SessionNameExpression)
		}
	}

	awsAuth := &api.Aws{
		Kind: &api.Aws_Implicit{
			Implicit: &api.AwsImplicit{},
		},
		ServiceName: serviceName,
		AssumeRole:  assumeRole,
		Region:      region,
	}
	if auth.SecretRef != nil && auth.SecretRef.Name != "" {
		if auth.AssumeRole != nil {
			errs = append(errs, errors.New("secretRef and assumeRole are mutually exclusive"))
		}
		// Get secret using the SecretIndex
		data, err := ctx.ResolveCredentialRef(*auth.SecretRef, namespace)
		if err != nil {
			errs = append(errs, err)
		} else {
			// Extract access key
			if value, exists := kubeutils.GetSecretDataValue(data, wellknown.AccessKey); !exists {
				errs = append(errs, errors.New("accessKey is missing or not a valid string"))
			} else {
				accessKeyId = value
			}

			// Extract secret key
			if value, exists := kubeutils.GetSecretDataValue(data, wellknown.SecretKey); !exists {
				errs = append(errs, errors.New("secretKey is missing or not a valid string"))
			} else {
				secretAccessKey = value
			}

			// Extract session token (optional)
			if value, exists := kubeutils.GetSecretDataValue(data, wellknown.SessionToken); exists {
				sessionToken = new(value)
			}
		}
		awsAuth.Kind = &api.Aws_ExplicitConfig{
			ExplicitConfig: &api.AwsExplicitConfig{
				AccessKeyId:     accessKeyId,
				SecretAccessKey: secretAccessKey,
				SessionToken:    sessionToken,
				Region:          "",
			},
		}
	}

	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_Aws{
			Aws: awsAuth,
		},
	}, errors.Join(errs...)
}

func buildAzureAuthPolicy(ctx PolicyCtx, auth *agentgateway.AzureAuth, namespace string) (*api.BackendAuthPolicy, error) {
	var errs []error
	if auth.SecretRef != nil {
		return buildAzureClientSecret(ctx, auth, namespace, errs)
	}

	if auth.ManagedIdentity != nil {
		uaid := &api.AzureManagedIdentityCredential_UserAssignedIdentity{}
		if auth.ManagedIdentity.ClientID != "" {
			uaid.Id = &api.AzureManagedIdentityCredential_UserAssignedIdentity_ClientId{
				ClientId: auth.ManagedIdentity.ClientID,
			}
		} else if auth.ManagedIdentity.ObjectID != "" {
			uaid.Id = &api.AzureManagedIdentityCredential_UserAssignedIdentity_ObjectId{
				ObjectId: auth.ManagedIdentity.ObjectID,
			}
		} else if auth.ManagedIdentity.ResourceID != "" {
			uaid.Id = &api.AzureManagedIdentityCredential_UserAssignedIdentity_ResourceId{
				ResourceId: auth.ManagedIdentity.ResourceID,
			}
		} else {
			errs = append(errs, errors.New("no valid User Assigned Identity identifier provided"))
			return nil, errors.Join(errs...)
		}
		return &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Azure{
				Azure: &api.Azure{
					Kind: &api.Azure_ExplicitConfig{
						ExplicitConfig: &api.AzureExplicitConfig{
							CredentialSource: &api.AzureExplicitConfig_ManagedIdentityCredential{
								ManagedIdentityCredential: &api.AzureManagedIdentityCredential{
									UserAssignedIdentity: uaid,
								},
							},
						},
					},
				},
			},
		}, nil
	}

	if auth.WorkloadIdentity != nil {
		return &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Azure{
				Azure: &api.Azure{
					Kind: &api.Azure_ExplicitConfig{
						ExplicitConfig: &api.AzureExplicitConfig{
							CredentialSource: &api.AzureExplicitConfig_WorkloadIdentityCredential{
								WorkloadIdentityCredential: &api.AzureWorkloadIdentityCredential{},
							},
						},
					},
				},
			},
		}, nil
	}

	// No explicit credential source: use implicit authentication
	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_Azure{
			Azure: &api.Azure{
				Kind: &api.Azure_Implicit{
					Implicit: &api.AzureImplicit{},
				},
			},
		},
	}, nil
}

func buildAzureClientSecret(ctx PolicyCtx, auth *agentgateway.AzureAuth, namespace string, errs []error) (*api.BackendAuthPolicy, error) {
	var clientID, tenantID, clientSecret string
	data, err := ctx.ResolveCredentialRef(*auth.SecretRef, namespace)
	if err != nil {
		errs = append(errs, err)
	} else {
		// Extract client ID
		if value, exists := kubeutils.GetSecretDataValue(data, wellknown.ClientID); !exists {
			errs = append(errs, errors.New("clientID is missing or not a valid string"))
		} else {
			clientID = value
		}

		// Extract tenant ID
		if value, exists := kubeutils.GetSecretDataValue(data, wellknown.TenantID); !exists {
			errs = append(errs, errors.New("tenantID is missing or not a valid string"))
		} else {
			tenantID = value
		}

		// Extract client secret
		if value, exists := kubeutils.GetSecretDataValue(data, wellknown.ClientSecret); !exists {
			errs = append(errs, errors.New("clientSecret is missing or not a valid string"))
		} else {
			clientSecret = value
		}
	}

	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_Azure{
			Azure: &api.Azure{
				Kind: &api.Azure_ExplicitConfig{
					ExplicitConfig: &api.AzureExplicitConfig{
						CredentialSource: &api.AzureExplicitConfig_ClientSecret{
							ClientSecret: &api.AzureClientSecret{
								ClientSecret: clientSecret,
								TenantId:     tenantID,
								ClientId:     clientID,
							},
						},
					},
				},
			},
		},
	}, errors.Join(errs...)
}

func buildGcpAuthPolicy(ctx PolicyCtx, auth *agentgateway.GcpAuth, namespace string) (*api.BackendAuthPolicy, error) {
	var errs []error
	var credential *string
	if auth.SecretRef != nil {
		// Preserve the user's explicit credential intent even when the Secret is
		// missing or malformed. An explicit empty credential fails in the proxy
		// instead of falling back to ambient GCP credentials.
		credential = new("")
		data, key, err := ctx.ResolveCredentialKeyRef(*auth.SecretRef, namespace, wellknown.GCPCredentialsJSON)
		if err != nil {
			errs = append(errs, err)
		} else if value, exists := kubeutils.GetSecretDataValue(data, key); !exists {
			errs = append(errs, fmt.Errorf("secret %s/%s missing %s value", namespace, auth.SecretRef.Name, key))
		} else {
			credential = &value
		}
	}

	gcp := &api.Gcp{
		Credential: credential,
	}
	if auth.Type == nil || *auth.Type == agentgateway.GcpAuthTypeAccessToken {
		gcp.TokenType = &api.Gcp_AccessToken_{
			AccessToken: &api.Gcp_AccessToken{},
		}
	} else {
		gcp.TokenType = &api.Gcp_IdToken_{
			IdToken: &api.Gcp_IdToken{
				Audience: auth.Audience,
			},
		}
	}
	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_Gcp{
			Gcp: gcp,
		},
	}, errors.Join(errs...)
}
