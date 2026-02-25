package plugins

import (
	"errors"
	"fmt"
	"strings"

	jsonpb "google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/types/known/durationpb"
	"google.golang.org/protobuf/types/known/structpb"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks_url"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/translator/sslutils"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

const (
	aiPolicySuffix                = ":ai"
	backendTlsPolicySuffix        = ":backend-tls"
	backendauthPolicySuffix       = ":backend-auth"
	tlsPolicySuffix               = ":tls"
	backendHttpPolicySuffix       = ":backend-http"
	mcpAuthorizationPolicySuffix  = ":mcp-authorization"
	mcpAuthenticationPolicySuffix = ":mcp-authentication"
)

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
	res, err := translateBackendPolicyToAgw(ctx, dummy, nil)
	return slices.MapFilter(res, func(e AgwPolicy) **api.BackendPolicySpec {
		return ptr.Of(e.Policy.GetBackend())
	}), err
}

func translateBackendPolicyToAgw(
	ctx PolicyCtx,
	policy *agentgateway.AgentgatewayPolicy,
	policyTarget *api.PolicyTarget,
) ([]AgwPolicy, error) {
	backend := policy.Spec.Backend
	if backend == nil {
		return nil, nil
	}
	agwPolicies := make([]AgwPolicy, 0)
	var errs []error

	policyName := getBackendPolicyName(policy.Namespace, policy.Name)

	if s := backend.HTTP; s != nil {
		pol := translateBackendHTTP(policy, policyTarget)
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := backend.TLS; s != nil {
		pol, err := translateBackendTLS(ctx, policy, policyTarget)
		if err != nil {
			logger.Error("error processing backend TLS", "err", err)
			errs = append(errs, err)
		}
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := backend.TCP; s != nil {
		pol, err := translateBackendTCP(ctx, policy, policyName, policyTarget)
		if err != nil {
			logger.Error("error processing backend TCP", "err", err)
			errs = append(errs, err)
		}
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := backend.MCP; s != nil {
		if backend.MCP.Authorization != nil {
			pol := translateBackendMCPAuthorization(policy, policyTarget)
			agwPolicies = append(agwPolicies, pol...)
		}

		if backend.MCP.Authentication != nil {
			pol, err := translateBackendMCPAuthentication(ctx, policy, policyTarget)
			if err != nil {
				logger.Error("error processing backend mcp auth", "err", err)
				errs = append(errs, err)
			}
			agwPolicies = append(agwPolicies, pol...)
		}
	}

	if s := backend.AI; s != nil {
		pol, err := translateBackendAI(ctx, policy, policyName, policyTarget)
		if err != nil {
			logger.Error("error processing backend AI", "err", err)
			errs = append(errs, err)
		}
		agwPolicies = append(agwPolicies, pol...)
	}

	if s := backend.Auth; s != nil {
		pol, err := translateBackendAuth(ctx, policy, policyName, policyTarget)
		if err != nil {
			logger.Error("error processing backend Auth", "err", err)
			errs = append(errs, err)
		}
		agwPolicies = append(agwPolicies, pol...)
	}

	return agwPolicies, errors.Join(errs...)
}

func translateBackendTCP(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, name string, target *api.PolicyTarget) ([]AgwPolicy, error) {
	// TODO
	return nil, nil
}

func translateBackendTLS(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, target *api.PolicyTarget) ([]AgwPolicy, error) {
	var errs []error
	tls := policy.Spec.Backend.TLS

	p := &api.BackendPolicySpec_BackendTLS{}

	if len(tls.MtlsCertificateRef) > 0 {
		// Currently we only support one, and enforce this in the API
		mtls := tls.MtlsCertificateRef[0]
		nn := types.NamespacedName{
			Namespace: policy.Namespace,
			Name:      mtls.Name,
		}
		scrt := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.Secrets, krt.FilterObjectName(nn)))
		if scrt == nil {
			errs = append(errs, fmt.Errorf("secret %s not found", nn))
		} else {
			if _, err := sslutils.ValidateTlsSecretData(nn.Name, nn.Namespace, scrt.Data); err != nil {
				errs = append(errs, fmt.Errorf("secret %v contains invalid certificate: %v", nn, err))
			}
			p.Cert = scrt.Data[corev1.TLSCertKey]
			p.Key = scrt.Data[corev1.TLSPrivateKeyKey]
			if ca, f := scrt.Data[corev1.ServiceAccountRootCAKey]; f {
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
			pem, err := sslutils.GetCACertFromConfigMap(ptr.Flatten(cfgmap))
			if err != nil {
				errs = append(errs, fmt.Errorf("error extracting CA cert from ConfigMap %s: %w", nn, err))
				continue
			}
			if sb.Len() > 0 {
				sb.WriteString("\n")
			}
			sb.WriteString(pem)
		}
		// TODO: if none, submit something to make agentgateway reject requests instead of fail open.
		if sb.Len() > 0 {
			p.Root = []byte(sb.String())
		}
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

	tlsPolicy := &api.Policy{
		Key:    policy.Namespace + "/" + policy.Name + tlsPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
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

	return []AgwPolicy{{Policy: tlsPolicy}}, errors.Join(errs...)
}

func translateBackendHTTP(policy *agentgateway.AgentgatewayPolicy, target *api.PolicyTarget) []AgwPolicy {
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
		Key:    policy.Namespace + "/" + policy.Name + backendHttpPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
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

	return []AgwPolicy{{Policy: tp}}
}

func translateBackendMCPAuthorization(policy *agentgateway.AgentgatewayPolicy, target *api.PolicyTarget) []AgwPolicy {
	backend := policy.Spec.Backend
	if backend == nil || backend.MCP == nil || backend.MCP.Authorization == nil {
		return nil
	}
	auth := backend.MCP.Authorization
	var allowPolicies, denyPolicies []string
	if auth.Action == shared.AuthorizationPolicyActionDeny {
		denyPolicies = append(denyPolicies, cast(auth.Policy.MatchExpressions)...)
	} else {
		allowPolicies = append(allowPolicies, cast(auth.Policy.MatchExpressions)...)
	}

	mcpPolicy := &api.Policy{
		Key:    policy.Namespace + "/" + policy.Name + mcpAuthorizationPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
		Kind: &api.Policy_Backend{
			Backend: &api.BackendPolicySpec{
				Kind: &api.BackendPolicySpec_McpAuthorization_{
					McpAuthorization: &api.BackendPolicySpec_McpAuthorization{
						Allow: allowPolicies,
						Deny:  denyPolicies,
					},
				},
			},
		},
	}

	logger.Debug("generated MCPBackend policy",
		"policy", policy.Name,
		"agentgateway_policy", mcpPolicy.Name)

	return []AgwPolicy{{Policy: mcpPolicy}}
}

func translateBackendMCPAuthentication(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, target *api.PolicyTarget) ([]AgwPolicy, error) {
	backend := policy.Spec.Backend
	if backend == nil || backend.MCP == nil || backend.MCP.Authentication == nil {
		return nil, nil
	}
	authnPolicy := backend.MCP.Authentication
	if authnPolicy == nil {
		return nil, nil
	}

	idp := api.BackendPolicySpec_McpAuthentication_UNSPECIFIED
	if authnPolicy.McpIDP != nil {
		if *authnPolicy.McpIDP == agentgateway.Keycloak {
			idp = api.BackendPolicySpec_McpAuthentication_KEYCLOAK
		} else if *authnPolicy.McpIDP == agentgateway.Auth0 {
			idp = api.BackendPolicySpec_McpAuthentication_AUTH0
		}
	}

	// default mode is Strict
	mode := api.BackendPolicySpec_McpAuthentication_STRICT
	if authnPolicy.Mode == agentgateway.JWTAuthenticationModeStrict {
		mode = api.BackendPolicySpec_McpAuthentication_STRICT
	} else if authnPolicy.Mode == agentgateway.JWTAuthenticationModePermissive {
		mode = api.BackendPolicySpec_McpAuthentication_PERMISSIVE
	} else if authnPolicy.Mode == agentgateway.JWTAuthenticationModeOptional {
		mode = api.BackendPolicySpec_McpAuthentication_OPTIONAL
	}

	jwksUrl, _, err := jwks_url.JwksUrlBuilderFactory().BuildJwksUrlAndTlsConfig(ctx.Krt, policy.Name, policy.Namespace, &authnPolicy.JWKS)
	if err != nil {
		logger.Error("failed resolving jwks url", "error", err)
		return nil, err
	}
	translatedInlineJwks, err := resolveRemoteJWKSInline(ctx, jwksUrl)
	if err != nil {
		logger.Error("failed resolving jwks", "jwks_uri", jwksUrl, "error", err)
		return nil, err
	}

	var errs []error
	var extraResourceMetadata map[string]*structpb.Value
	for k, v := range authnPolicy.ResourceMetadata {
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

	mcpAuthn := &api.BackendPolicySpec_McpAuthentication{
		Issuer:    authnPolicy.Issuer,
		Audiences: authnPolicy.Audiences,
		Provider:  idp,
		ResourceMetadata: &api.BackendPolicySpec_McpAuthentication_ResourceMetadata{
			Extra: extraResourceMetadata,
		},
		JwksInline: translatedInlineJwks,
		Mode:       mode,
	}
	mcpAuthnPolicy := &api.Policy{
		Key:    policy.Namespace + "/" + policy.Name + mcpAuthenticationPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
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

	return []AgwPolicy{{Policy: mcpAuthnPolicy}}, errors.Join(errs...)
}

// translateBackendAI processes AI configuration and creates corresponding Agw policies
func translateBackendAI(ctx PolicyCtx, agwPolicy *agentgateway.AgentgatewayPolicy, name string, policyTarget *api.PolicyTarget) ([]AgwPolicy, error) {
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
		translatedAIPolicy.PromptCaching.MinTokens = ptr.Of(uint32(aiSpec.PromptCaching.MinTokens)) //nolint:gosec // G115: MinTokens is validated by kubebuilder to be >= 0
	}

	if aiSpec.Routes != nil {
		r := make(map[string]api.BackendPolicySpec_Ai_RouteType)
		for path, routeType := range aiSpec.Routes {
			r[path] = translateRouteType(routeType)
		}
		translatedAIPolicy.Routes = r
	}

	aiPolicy := &api.Policy{
		Key:    name + aiPolicySuffix + attachmentName(policyTarget),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, agwPolicy),
		Target: policyTarget,
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

	return []AgwPolicy{{Policy: aiPolicy}}, errors.Join(errs...)
}

func translateBackendAuth(ctx PolicyCtx, policy *agentgateway.AgentgatewayPolicy, name string, target *api.PolicyTarget) ([]AgwPolicy, error) {
	var errs []error
	auth := policy.Spec.Backend.Auth

	if auth == nil {
		return nil, nil
	}

	var translatedAuth *api.BackendAuthPolicy

	if auth.InlineKey != nil && *auth.InlineKey != "" {
		translatedAuth = &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Key{
				Key: &api.Key{Secret: *auth.InlineKey},
			},
		}
	} else if auth.SecretRef != nil {
		// Resolve secret and extract Authorization value
		secret, err := kubeutils.GetSecret(ctx.Collections.Secrets, ctx.Krt, auth.SecretRef.Name, policy.Namespace)
		if err != nil {
			errs = append(errs, err)
		} else {
			if authKey, ok := kubeutils.GetSecretAuth(secret); ok {
				translatedAuth = &api.BackendAuthPolicy{
					Kind: &api.BackendAuthPolicy_Key{
						Key: &api.Key{Secret: authKey},
					},
				}
			} else {
				errs = append(errs, fmt.Errorf("secret %s/%s missing Authorization value", policy.Namespace, auth.SecretRef.Name))
			}
		}
	} else if auth.AWS != nil {
		awsAuth, err := buildAwsAuthPolicy(ctx.Krt, auth.AWS, ctx.Collections.Secrets, policy.Namespace)
		translatedAuth = awsAuth
		errs = append(errs, err)
	} else if auth.Azure != nil {
		azureAuth, err := buildAzureAuthPolicy(ctx.Krt, auth.Azure, ctx.Collections.Secrets, policy.Namespace)
		translatedAuth = azureAuth
		errs = append(errs, err)
	} else if auth.GCP != nil {
		translatedAuth = buildGcpAuthPolicy(auth.GCP)
	} else if auth.Passthrough != nil {
		translatedAuth = &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Passthrough{
				Passthrough: &api.Passthrough{},
			},
		}
	}

	if translatedAuth == nil {
		return nil, errors.Join(errs...)
	}

	authPolicy := &api.Policy{
		Key:    name + backendauthPolicySuffix + attachmentName(target),
		Name:   TypedResourceName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Target: target,
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

	return []AgwPolicy{{Policy: authPolicy}}, errors.Join(errs...)
}

// translateRouteType converts kgateway RouteType to agentgateway proto RouteType
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
	case agentgateway.RouteTypeResponses:
		return api.BackendPolicySpec_Ai_RESPONSES
	case agentgateway.RouteTypeAnthropicTokenCount:
		return api.BackendPolicySpec_Ai_ANTHROPIC_TOKEN_COUNT
	case agentgateway.RouteTypeEmbeddings:
		return api.BackendPolicySpec_Ai_EMBEDDINGS
	case agentgateway.RouteTypeRealtime:
		return api.BackendPolicySpec_Ai_REALTIME
	default:
		// Default to completions if unknown type
		return api.BackendPolicySpec_Ai_COMPLETIONS
	}
}

func buildAwsAuthPolicy(krtctx krt.HandlerContext, auth *agentgateway.AwsAuth, secrets krt.Collection[*corev1.Secret], namespace string) (*api.BackendAuthPolicy, error) {
	var errs []error
	if auth == nil {
		logger.Warn("using implicit AWS auth for AI backend")
		return &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Aws{
				Aws: &api.Aws{
					Kind: &api.Aws_Implicit{
						Implicit: &api.AwsImplicit{},
					},
				},
			},
		}, nil
	}

	if auth.SecretRef.Name == "" {
		logger.Warn("not using any auth for AWS - it's most likely not what you want")
		return nil, nil
	}

	// Get secret using the SecretIndex
	secret, err := kubeutils.GetSecret(secrets, krtctx, auth.SecretRef.Name, namespace)
	if err != nil {
		// Return nil auth policy if secret not found - this will be handled upstream
		// TODO(npolshak): Add backend status errors https://github.com/kgateway-dev/kgateway/issues/11966
		return nil, err
	}

	var accessKeyId, secretAccessKey string
	var sessionToken *string

	// Extract access key
	if value, exists := kubeutils.GetSecretValue(secret, wellknown.AccessKey); !exists {
		errs = append(errs, errors.New("accessKey is missing or not a valid string"))
	} else {
		accessKeyId = value
	}

	// Extract secret key
	if value, exists := kubeutils.GetSecretValue(secret, wellknown.SecretKey); !exists {
		errs = append(errs, errors.New("secretKey is missing or not a valid string"))
	} else {
		secretAccessKey = value
	}

	// Extract session token (optional)
	if value, exists := kubeutils.GetSecretValue(secret, wellknown.SessionToken); exists {
		sessionToken = ptr.Of(value)
	}

	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_Aws{
			Aws: &api.Aws{
				Kind: &api.Aws_ExplicitConfig{
					ExplicitConfig: &api.AwsExplicitConfig{
						AccessKeyId:     accessKeyId,
						SecretAccessKey: secretAccessKey,
						SessionToken:    sessionToken,
						Region:          "",
					},
				},
			},
		},
	}, errors.Join(errs...)
}

func buildAzureAuthPolicy(krtctx krt.HandlerContext, auth *agentgateway.AzureAuth, secrets krt.Collection[*corev1.Secret], namespace string) (*api.BackendAuthPolicy, error) {
	var errs []error
	if auth == nil {
		logger.Warn("using implicit Azure auth for AI backend")
		return &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Azure{
				Azure: &api.Azure{
					Kind: &api.Azure_DeveloperImplicit{
						DeveloperImplicit: &api.AzureDeveloperImplicit{},
					},
				},
			},
		}, nil
	}

	if auth.SecretRef.Name != "" {
		return buildAzureClientSecret(secrets, krtctx, auth, namespace, errs)
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

	errs = append(errs, errors.New("no valid Azure auth method provided"))
	return nil, errors.Join(errs...)
}

func buildAzureClientSecret(secrets krt.Collection[*corev1.Secret], krtctx krt.HandlerContext, auth *agentgateway.AzureAuth, namespace string, errs []error) (*api.BackendAuthPolicy, error) {
	secret, err := kubeutils.GetSecret(secrets, krtctx, auth.SecretRef.Name, namespace)
	if err != nil {
		// Return nil auth policy if secret not found - this will be handled upstream
		// TODO(npolshak): Add backend status errors https://github.com/kgateway-dev/kgateway/issues/11966
		return nil, err
	}

	var clientID, tenantID, clientSecret string

	// Extract client ID
	if value, exists := kubeutils.GetSecretValue(secret, wellknown.ClientID); !exists {
		errs = append(errs, errors.New("clientID is missing or not a valid string"))
	} else {
		clientID = value
	}

	// Extract tenant ID
	if value, exists := kubeutils.GetSecretValue(secret, wellknown.TenantID); !exists {
		errs = append(errs, errors.New("tenantID is missing or not a valid string"))
	} else {
		tenantID = value
	}

	// Extract client secret
	if value, exists := kubeutils.GetSecretValue(secret, wellknown.ClientSecret); !exists {
		errs = append(errs, errors.New("clientSecret is missing or not a valid string"))
	} else {
		clientSecret = value
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

func buildGcpAuthPolicy(auth *agentgateway.GcpAuth) *api.BackendAuthPolicy {
	if auth.Type == nil || *auth.Type == agentgateway.GcpAuthTypeAccessToken {
		return &api.BackendAuthPolicy{
			Kind: &api.BackendAuthPolicy_Gcp{
				Gcp: &api.Gcp{
					TokenType: &api.Gcp_AccessToken_{
						AccessToken: &api.Gcp_AccessToken{},
					},
				},
			},
		}
	}
	return &api.BackendAuthPolicy{
		Kind: &api.BackendAuthPolicy_Gcp{
			Gcp: &api.Gcp{
				TokenType: &api.Gcp_IdToken_{
					IdToken: &api.Gcp_IdToken{
						Audience: auth.Audience,
					},
				},
			},
		},
	}
}
