package plugins

import (
	"fmt"

	"istio.io/istio/pkg/slices"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	oidcpkg "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const oidcPolicySuffix = ":oidc"

const (
	oidcConfigTokenEndpointAuthMethodClientSecretBasic   = "ClientSecretBasic"
	oidcConfigTokenEndpointAuthMethodClientSecretPost    = "ClientSecretPost"
	oidcConfigTokenEndpointAuthMethodNone                = "None"
	oidcProviderTokenEndpointAuthMethodClientSecretBasic = "client_secret_basic"
	oidcProviderTokenEndpointAuthMethodClientSecretPost  = "client_secret_post"
	oidcProviderTokenEndpointAuthMethodNone              = "none"
)

// oidcPolicyIDForPolicyKey returns the canonical xDS PolicyId for an emitted
// xDS Policy key.
func oidcPolicyIDForPolicyKey(key string) string {
	return "policy/" + key
}

func processOIDCPolicy(
	ctx PolicyCtx,
	oidcCfg *agentgateway.OIDC,
	policyNSN types.NamespacedName,
	policyKey string,
	oidcLookup oidcpkg.Lookup,
) (*api.Policy, error) {
	if oidcLookup == nil {
		return nil, fmt.Errorf("oidc lookup is not configured")
	}

	owner, ok := oidcpkg.PolicyOIDCLookupOwner(policyNSN.Namespace, policyNSN.Name, oidcCfg)
	if !ok {
		return nil, fmt.Errorf("could not derive OIDC owner for policy")
	}

	provider, err := oidcLookup.ResolveForOwner(ctx.Krt, owner)
	if err != nil {
		return nil, err
	}

	if provider.JwksInline == "" {
		return nil, fmt.Errorf("oidc provider for %q has no jwks_inline (fetch pending)", oidcCfg.IssuerURL)
	}

	var clientSecret string
	if oidcCfg.ClientSecret != nil {
		secret, err := resolveOIDCClientSecret(ctx, policyNSN.Namespace, oidcCfg)
		if err != nil {
			return nil, err
		}
		clientSecret = secret
	}

	tokenAuth, err := resolveOIDCTokenEndpointAuth(oidcCfg, provider, clientSecret != "")
	if err != nil {
		return nil, err
	}

	// OIDC phase is always Gateway in xDS IR.
	return &api.Policy{
		Key: policyKey + oidcPolicySuffix,
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Phase: api.TrafficPolicySpec_GATEWAY,
				Kind: &api.TrafficPolicySpec_Oidc{
					Oidc: &api.TrafficPolicySpec_OIDC{
						PolicyId:              oidcPolicyIDForPolicyKey(policyKey + oidcPolicySuffix),
						Issuer:                provider.IssuerURL,
						AuthorizationEndpoint: provider.AuthorizationEndpoint,
						TokenEndpoint:         provider.TokenEndpoint,
						JwksInline:            provider.JwksInline,
						ClientId:              oidcCfg.ClientID,
						ClientSecret:          clientSecret,
						RedirectUri:           oidcCfg.RedirectURI,
						Scopes:                normalizedOIDCScopes(oidcCfg.Scopes),
						TokenEndpointAuth:     tokenAuth,
					},
				},
			},
		},
	}, nil
}

func normalizedOIDCScopes(input []string) []string {
	out := []string{"openid"}
	seen := map[string]struct{}{"openid": {}}
	for _, s := range input {
		if _, ok := seen[s]; !ok {
			out = append(out, s)
			seen[s] = struct{}{}
		}
	}
	return out
}

func resolveOIDCClientSecret(ctx PolicyCtx, policyNamespace string, oidcCfg *agentgateway.OIDC) (string, error) {
	if oidcCfg.ClientSecret == nil {
		return "", nil
	}

	secret, err := kubeutils.GetSecret(ctx.Collections.Secrets, ctx.Krt, oidcCfg.ClientSecret.Name, policyNamespace)
	if err != nil {
		return "", fmt.Errorf("oidc clientSecret %q: %w", oidcCfg.ClientSecret.Name, err)
	}

	// kubeutils.GetSecretValue trims whitespace, so a whitespace-only Data
	// value returns ("", true). Reject explicitly: an empty clientSecret
	// would silently switch us onto the public-client auth path.
	value, ok := kubeutils.GetSecretValue(secret, wellknown.ClientSecret)
	if !ok || value == "" {
		return "", fmt.Errorf("oidc clientSecret %q is missing or has empty %q value", oidcCfg.ClientSecret.Name, wellknown.ClientSecret)
	}
	return value, nil
}

func resolveOIDCTokenEndpointAuth(
	oidcCfg *agentgateway.OIDC,
	provider *oidcpkg.DiscoveredProvider,
	hasClientSecret bool,
) (api.TrafficPolicySpec_OIDC_TokenEndpointAuth, error) {
	if oidcCfg.TokenEndpointAuthMethod != nil {
		return configuredOIDCTokenEndpointAuth(oidcCfg, hasClientSecret)
	}
	return discoveredOIDCTokenEndpointAuth(provider.TokenEndpointAuthMethodsSupported, hasClientSecret)
}

func configuredOIDCTokenEndpointAuth(
	oidcCfg *agentgateway.OIDC,
	hasClientSecret bool,
) (api.TrafficPolicySpec_OIDC_TokenEndpointAuth, error) {
	switch *oidcCfg.TokenEndpointAuthMethod {
	case oidcConfigTokenEndpointAuthMethodClientSecretBasic:
		if !hasClientSecret {
			return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf("tokenEndpointAuthMethod %s requires a clientSecret", oidcConfigTokenEndpointAuthMethodClientSecretBasic)
		}
		return api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC, nil
	case oidcConfigTokenEndpointAuthMethodClientSecretPost:
		if !hasClientSecret {
			return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf("tokenEndpointAuthMethod %s requires a clientSecret", oidcConfigTokenEndpointAuthMethodClientSecretPost)
		}
		return api.TrafficPolicySpec_OIDC_CLIENT_SECRET_POST, nil
	case oidcConfigTokenEndpointAuthMethodNone:
		if hasClientSecret {
			return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf("tokenEndpointAuthMethod %s must not be paired with a clientSecret", oidcConfigTokenEndpointAuthMethodNone)
		}
		return api.TrafficPolicySpec_OIDC_NONE, nil
	default:
		return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf("unsupported tokenEndpointAuthMethod %q", *oidcCfg.TokenEndpointAuthMethod)
	}
}

func discoveredOIDCTokenEndpointAuth(
	methods []string,
	hasClientSecret bool,
) (api.TrafficPolicySpec_OIDC_TokenEndpointAuth, error) {
	// OIDC Discovery §4.2 / RFC 8414 §2: when the IdP omits
	// `token_endpoint_auth_methods_supported`, the spec-defined default is
	// `client_secret_basic`. Public clients still surface an explicit error
	// below rather than silently programming an unauthenticated POST.
	if len(methods) == 0 {
		methods = []string{oidcProviderTokenEndpointAuthMethodClientSecretBasic}
	}

	if hasClientSecret {
		if slices.Contains(methods, oidcProviderTokenEndpointAuthMethodClientSecretBasic) {
			return api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC, nil
		}
		if slices.Contains(methods, oidcProviderTokenEndpointAuthMethodClientSecretPost) {
			return api.TrafficPolicySpec_OIDC_CLIENT_SECRET_POST, nil
		}
		return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf("IdP does not advertise a supported confidential auth method")
	}

	if slices.Contains(methods, oidcProviderTokenEndpointAuthMethodNone) {
		return api.TrafficPolicySpec_OIDC_NONE, nil
	}
	return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf("OIDC client has no clientSecret but IdP does not advertise %q auth", oidcProviderTokenEndpointAuthMethodNone)
}
