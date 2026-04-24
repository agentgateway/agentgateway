package plugins

import (
	"bytes"
	"fmt"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	oidcpkg "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/oidc"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

const oidcPolicySuffix = ":oidc"

// Data key inside the `.spec.traffic.oidc.clientSecret` Secret from which the
// controller extracts the OAuth2 client secret. Mirrors the BasicAuth ".htaccess"
// convention (see processBasicAuthenticationPolicy).
const oidcClientSecretDataKey = "clientSecret"

const (
	oidcConfigTokenEndpointAuthMethodClientSecretBasic   = "ClientSecretBasic"
	oidcConfigTokenEndpointAuthMethodClientSecretPost    = "ClientSecretPost"
	oidcConfigTokenEndpointAuthMethodNone                = "None"
	oidcProviderTokenEndpointAuthMethodClientSecretBasic = "client_secret_basic"
	oidcProviderTokenEndpointAuthMethodClientSecretPost  = "client_secret_post"
	oidcProviderTokenEndpointAuthMethodNone              = "none"
)

// policyIDForPolicy returns the canonical xDS PolicyId for an
// AgentgatewayPolicy, encoded as `policy/<namespace>/<name>`. The dataplane
// parses this with `http::oidc::PolicyId::from_str` (see
// crates/agentgateway/src/http/oidc/mod.rs).
func policyIDForPolicy(namespace, name string) string {
	return fmt.Sprintf("policy/%s/%s", namespace, name)
}

func processOIDCPolicy(
	ctx PolicyCtx,
	oidcCfg *agentgateway.OIDC,
	policyPhase *agentgateway.PolicyPhase,
	basePolicyName string,
	policy types.NamespacedName,
	oidcLookup oidcpkg.Lookup,
) (*api.Policy, error) {
	clientSecret, err := resolveOIDCClientSecret(ctx, oidcCfg, policy.Namespace)
	if err != nil {
		return nil, err
	}
	hasClientSecret := clientSecret != ""

	if oidcLookup == nil {
		return nil, fmt.Errorf("oidc lookup is not configured")
	}

	owner, ok := oidcpkg.PolicyOIDCLookupOwner(policy.Namespace, policy.Name, oidcCfg)
	if !ok {
		return nil, fmt.Errorf("oidc lookup owner is not configured")
	}
	provider, err := oidcLookup.ResolveForOwner(ctx.Krt, owner)
	if err != nil {
		return nil, fmt.Errorf("oidc provider for %s/%s not available: %w", policy.Namespace, policy.Name, err)
	}
	if provider == nil {
		return nil, fmt.Errorf("oidc provider for %s/%s not yet fetched", policy.Namespace, policy.Name)
	}

	tokenEndpointAuth, err := resolvedOIDCTokenEndpointAuth(oidcCfg, provider, hasClientSecret)
	if err != nil {
		return nil, fmt.Errorf("oidc token endpoint auth for %s/%s is invalid: %w", policy.Namespace, policy.Name, err)
	}

	spec := &api.TrafficPolicySpec_OIDC{
		PolicyId:              policyIDForPolicy(policy.Namespace, policy.Name),
		Issuer:                provider.IssuerURL,
		AuthorizationEndpoint: provider.AuthorizationEndpoint,
		TokenEndpoint:         provider.TokenEndpoint,
		TokenEndpointAuth:     tokenEndpointAuth,
		JwksInline:            provider.JwksJSON,
		ClientId:              oidcCfg.ClientID,
		ClientSecret:          clientSecret,
		RedirectUri:           oidcCfg.RedirectURI,
		Scopes:                oidcCfg.Scopes,
	}

	oidcPolicy := &api.Policy{
		Key:  basePolicyName + oidcPolicySuffix,
		Name: TypedResourceFromName(wellknown.AgentgatewayPolicyGVK.Kind, policy),
		Kind: &api.Policy_Traffic{
			Traffic: &api.TrafficPolicySpec{
				Phase: phase(policyPhase),
				Kind:  &api.TrafficPolicySpec_Oidc{Oidc: spec},
			},
		},
	}

	logger.Debug("generated oidc policy",
		"policy", basePolicyName,
		"agentgateway_policy", oidcPolicy.Name)

	return oidcPolicy, nil
}

// resolveOIDCClientSecret reads the `clientSecret` data key from the Kubernetes
// Secret referenced by the policy, if any. Returns the empty string for public
// clients (ClientSecret unset). Matches the BasicAuth/APIKey pattern of using
// krt.FetchOne against ctx.Collections.Secrets.
//
// Whitespace is used only for emptiness detection; the original bytes are
// returned unchanged so that an IdP-issued secret containing leading or
// trailing whitespace is preserved byte-for-byte.
func resolveOIDCClientSecret(
	ctx PolicyCtx,
	oidcCfg *agentgateway.OIDC,
	namespace string,
) (string, error) {
	if oidcCfg.ClientSecret == nil {
		return "", nil
	}
	if ctx.Collections == nil {
		return "", fmt.Errorf("oidc clientSecret resolution requires a Secrets collection")
	}
	scrt := ptr.Flatten(krt.FetchOne(
		ctx.Krt,
		ctx.Collections.Secrets,
		krt.FilterKey(namespace+"/"+oidcCfg.ClientSecret.Name),
	))
	if scrt == nil {
		return "", fmt.Errorf("oidc clientSecret %q not found", oidcCfg.ClientSecret.Name)
	}
	value, ok := scrt.Data[oidcClientSecretDataKey]
	if !ok {
		return "", fmt.Errorf(
			"oidc clientSecret %q is missing required data key %q",
			oidcCfg.ClientSecret.Name,
			oidcClientSecretDataKey,
		)
	}
	if len(bytes.TrimSpace(value)) == 0 {
		return "", fmt.Errorf(
			"oidc clientSecret %q has an empty %q value",
			oidcCfg.ClientSecret.Name,
			oidcClientSecretDataKey,
		)
	}
	// Preserve the original byte value: the IdP's secret may legitimately
	// contain whitespace at its boundaries.
	return string(value), nil
}

// configuredOIDCTokenEndpointAuth maps the user-selected token-endpoint auth
// method to its xDS enum value, enforcing that the method agrees with whether
// a client secret has been supplied: `None` requires no secret;
// `ClientSecretBasic`/`ClientSecretPost` require one. The caller must gate on
// `oidcCfg.TokenEndpointAuthMethod != nil`; when it is nil the method is
// derived from IdP discovery instead (see `discoveredOIDCTokenEndpointAuth`).
func configuredOIDCTokenEndpointAuth(
	oidcCfg *agentgateway.OIDC,
	hasClientSecret bool,
) (api.TrafficPolicySpec_OIDC_TokenEndpointAuth, error) {
	switch *oidcCfg.TokenEndpointAuthMethod {
	case oidcConfigTokenEndpointAuthMethodClientSecretBasic:
		if !hasClientSecret {
			return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf(
				"tokenEndpointAuthMethod %s requires a clientSecret",
				oidcConfigTokenEndpointAuthMethodClientSecretBasic,
			)
		}
		return api.TrafficPolicySpec_OIDC_CLIENT_SECRET_BASIC, nil
	case oidcConfigTokenEndpointAuthMethodClientSecretPost:
		if !hasClientSecret {
			return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf(
				"tokenEndpointAuthMethod %s requires a clientSecret",
				oidcConfigTokenEndpointAuthMethodClientSecretPost,
			)
		}
		return api.TrafficPolicySpec_OIDC_CLIENT_SECRET_POST, nil
	case oidcConfigTokenEndpointAuthMethodNone:
		if hasClientSecret {
			return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf(
				"tokenEndpointAuthMethod %s must not be paired with a clientSecret",
				oidcConfigTokenEndpointAuthMethodNone,
			)
		}
		return api.TrafficPolicySpec_OIDC_NONE, nil
	default:
		return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf(
			"unsupported tokenEndpointAuthMethod %q",
			*oidcCfg.TokenEndpointAuthMethod,
		)
	}
}

func resolvedOIDCTokenEndpointAuth(
	oidcCfg *agentgateway.OIDC,
	provider *oidcpkg.DiscoveredProvider,
	hasClientSecret bool,
) (api.TrafficPolicySpec_OIDC_TokenEndpointAuth, error) {
	if oidcCfg.TokenEndpointAuthMethod != nil {
		return configuredOIDCTokenEndpointAuth(oidcCfg, hasClientSecret)
	}
	return discoveredOIDCTokenEndpointAuth(provider.TokenEndpointAuthMethodsSupported, hasClientSecret)
}

// discoveredOIDCTokenEndpointAuth picks a method from the IdP-advertised list
// given whether a client secret is present. Mirrors
// crates/agentgateway/src/http/oauth.rs::parse_token_endpoint_auth_methods so
// the control-plane and dataplane reach identical conclusions from the same
// discovery document.
func discoveredOIDCTokenEndpointAuth(
	methods []string,
	hasClientSecret bool,
) (api.TrafficPolicySpec_OIDC_TokenEndpointAuth, error) {
	// Per OIDC Discovery §4.2 and RFC 8414 §2, the spec-defined default when
	// the IdP omits token_endpoint_auth_methods_supported is
	// client_secret_basic. Substitute that default here so confidential
	// clients still pick basic while secretless clients fall through to the
	// "does not advertise 'none'" error below instead of silently picking
	// public-client mode.
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
		return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf(
			"IdP does not advertise %q or %q; register the client as public (omit clientSecret and set tokenEndpointAuthMethod to %s) or use a supported confidential method",
			oidcProviderTokenEndpointAuthMethodClientSecretBasic,
			oidcProviderTokenEndpointAuthMethodClientSecretPost,
			oidcConfigTokenEndpointAuthMethodNone,
		)
	}

	if slices.Contains(methods, oidcProviderTokenEndpointAuthMethodNone) {
		return api.TrafficPolicySpec_OIDC_NONE, nil
	}
	return api.TrafficPolicySpec_OIDC_TOKEN_ENDPOINT_AUTH_UNSPECIFIED, fmt.Errorf(
		"OIDC client has no clientSecret but IdP does not advertise %q auth; register the client as public at the IdP or provide a clientSecret",
		oidcProviderTokenEndpointAuthMethodNone,
	)
}
