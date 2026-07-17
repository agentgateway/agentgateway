package plugins

import (
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

func oauthTestPolicyCtx(t *testing.T, secrets ...*corev1.Secret) PolicyCtx {
	t.Helper()
	return oauthTestPolicyCtxWithBackend(t, func(krt.HandlerContext, string, schema.GroupKind, gwv1.ObjectName, *gwv1.Namespace, *gwv1.PortNumber) (*api.BackendReference, error) {
		return &api.BackendReference{
			Kind: &api.BackendReference_Backend{
				Backend: "default/token-endpoint",
			},
		}, nil
	}, secrets...)
}

func oauthTestPolicyCtxWithBackend(
	t *testing.T,
	policyBackend func(krt.HandlerContext, string, schema.GroupKind, gwv1.ObjectName, *gwv1.Namespace, *gwv1.PortNumber) (*api.BackendReference, error),
	secrets ...*corev1.Secret,
) PolicyCtx {
	t.Helper()
	secretCollection := krt.NewStaticCollection[*corev1.Secret](nil, secrets, krt.WithName("plugins/oauthTestPolicyCtx"))
	return PolicyCtx{
		Krt:         krt.TestingDummyContext{},
		Collections: &AgwCollections{},
		References: BuildReferenceIndex(nil, nil, ReferenceTypes{
			PolicyBackend: policyBackend,
		}),
		CredentialResolver: kubeutils.NewSecretCredentialResolver(secretCollection),
	}
}

func oauthTokenEndpointRef() gwv1.BackendObjectReference {
	return gwv1.BackendObjectReference{
		Group: ptr.Of(gwv1.Group("agentgateway.dev")),
		Kind:  ptr.Of(gwv1.Kind("AgentgatewayBackend")),
		Name:  "token-endpoint",
	}
}

func TestOAuthTokenExchangeTokenEndpointIsReferencedBackend(t *testing.T) {
	policy := &agentgateway.AgentgatewayPolicy{
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					Auth: &agentgateway.BackendAuth{
						OAuthTokenExchange: &agentgateway.OAuthTokenExchange{
							BackendRef: oauthTokenEndpointRef(),
						},
					},
				},
			},
		},
	}

	refs := referencedBackendRefsFromPolicy(policy)
	if len(refs) != 1 {
		t.Fatalf("referenced backend refs length = %d, want 1", len(refs))
	}
	ref := refs[0]
	if ref.Name != "token-endpoint" ||
		ref.Group == nil || *ref.Group != "agentgateway.dev" ||
		ref.Kind == nil || *ref.Kind != "AgentgatewayBackend" {
		t.Fatalf("referenced backend ref = %+v, want token endpoint AgentgatewayBackend", ref)
	}
}

func TestBuildOAuthTokenExchangeResolvesTokenEndpointWhenNil(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)

	oauth, err := BuildOAuthTokenExchange(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
	}, "default", nil)
	if err != nil {
		t.Fatalf("BuildOAuthTokenExchange() error = %v, want nil", err)
	}
	if got := oauth.GetTokenEndpoint().GetBackend(); got != "default/token-endpoint" {
		t.Fatalf("token endpoint backend = %q, want default/token-endpoint", got)
	}
}

func TestBuildOAuthTokenExchangeRejectsNilAuth(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)

	oauth, err := BuildOAuthTokenExchange(ctx, nil, "default", nil)
	want := "oauthTokenExchange must not be nil"
	if err == nil || err.Error() != want {
		t.Fatalf("BuildOAuthTokenExchange() error = %v, want %q", err, want)
	}
	if oauth != nil {
		t.Fatalf("BuildOAuthTokenExchange() oauth = %v, want nil", oauth)
	}
}

func TestBuildOAuthTokenExchangeSuppliedTokenEndpointPreservesValidationErrors(t *testing.T) {
	var calls int
	ctx := oauthTestPolicyCtxWithBackend(t, func(krt.HandlerContext, string, schema.GroupKind, gwv1.ObjectName, *gwv1.Namespace, *gwv1.PortNumber) (*api.BackendReference, error) {
		calls++
		return nil, nil
	})
	tokenEndpoint := &api.BackendReference{
		Kind: &api.BackendReference_Backend{
			Backend: "default/prebuilt-token-endpoint",
		},
	}

	oauth, err := BuildOAuthTokenExchange(ctx, &agentgateway.OAuthTokenExchange{
		SubjectToken: &agentgateway.OAuthTokenSpec{
			Source: &agentgateway.AuthorizationExtractionLocation{
				Expression: ptr.Of(agentgateway.CELExpression("((")),
			},
		},
	}, "default", tokenEndpoint)
	want := "oauth subjectToken source expression is not a valid CEL expression"
	if err == nil || !strings.Contains(err.Error(), want) {
		t.Fatalf("BuildOAuthTokenExchange() error = %v, want containing %q", err, want)
	}
	if calls != 0 {
		t.Fatalf("backend ref resolution calls = %d, want 0", calls)
	}
	if oauth == nil {
		t.Fatal("BuildOAuthTokenExchange() oauth = nil, want partial object")
	}
	if oauth.TokenEndpoint != tokenEndpoint {
		t.Fatalf("token endpoint = %p, want supplied endpoint %p", oauth.TokenEndpoint, tokenEndpoint)
	}
	if got := oauth.GetSubjectToken().GetSource().GetExpression(); got != "((" {
		t.Fatalf("subject token expression = %q, want invalid expression preserved", got)
	}
}

func TestOAuthTokenExchangeClientAuthPublicClientRequiresPost(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)

	policy, err := buildOAuthTokenExchangePolicy(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
		ClientAuth: &agentgateway.OAuthClientAuth{
			ClientID: "public-client",
			Method:   ptr.Of(agentgateway.OAuthClientAuthMethodClientSecretPost),
		},
	}, "default")
	if err != nil {
		t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want nil", err)
	}
	clientAuth := policy.GetOauthTokenExchange().GetClientAuth()
	if clientAuth.GetMethod() != api.OAuthClientAuth_CLIENT_SECRET_POST {
		t.Fatalf("client auth method = %v, want CLIENT_SECRET_POST", clientAuth.GetMethod())
	}
	if clientAuth.ClientSecret != nil {
		t.Fatalf("client secret = %q, want nil", clientAuth.GetClientSecret())
	}

	_, err = buildOAuthTokenExchangePolicy(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
		ClientAuth: &agentgateway.OAuthClientAuth{
			ClientID: "public-client",
		},
	}, "default")
	if err == nil || !strings.Contains(err.Error(), "without secretRef requires method ClientSecretPost or PrivateKeyJwt") {
		t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want public client method error", err)
	}
}

func TestOAuthTokenExchangeClientAuthMissingSecretKeyPreservesExplicitSecretIntent(t *testing.T) {
	ctx := oauthTestPolicyCtx(t, &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "oauth-client",
		},
		Data: map[string][]byte{
			"other": []byte("value"),
		},
	})

	policy, err := buildOAuthTokenExchangePolicy(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
		ClientAuth: &agentgateway.OAuthClientAuth{
			ClientID: "gateway",
			SecretRef: &agentgateway.LocalSecretKeyRef{
				Name: "oauth-client",
			},
		},
	}, "default")
	if err == nil || !strings.Contains(err.Error(), "missing clientSecret value") {
		t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want missing clientSecret error", err)
	}

	clientAuth := policy.GetOauthTokenExchange().GetClientAuth()
	if clientAuth.ClientSecret == nil {
		t.Fatal("client secret is nil, want explicit empty secret")
	}
	if got := clientAuth.GetClientSecret(); got != "" {
		t.Fatalf("client secret = %q, want empty", got)
	}
}

func TestOAuthTokenExchangeClientAuthPrivateKeyJWT(t *testing.T) {
	ctx := oauthTestPolicyCtx(t, &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "oauth-signing-key",
		},
		Data: map[string][]byte{
			"signingKey": []byte("-----BEGIN PRIVATE KEY-----\nkey\n-----END PRIVATE KEY-----"),
		},
	})

	policy, err := buildOAuthTokenExchangePolicy(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
		ClientAuth: &agentgateway.OAuthClientAuth{
			ClientID: "gateway",
			Method:   ptr.Of(agentgateway.OAuthClientAuthMethodPrivateKeyJWT),
			PrivateKeyJWT: &agentgateway.OAuthPrivateKeyJWT{
				SigningKeyRef: agentgateway.LocalSecretKeyRef{
					Name: "oauth-signing-key",
				},
				Alg:               ptr.Of(agentgateway.OAuthPrivateKeyJWTSigningAlgorithmES256),
				KeyID:             new("kid-1"),
				AssertionAudience: "https://issuer.example.com/oauth/token",
			},
		},
	}, "default")
	if err != nil {
		t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want nil", err)
	}

	clientAuth := policy.GetOauthTokenExchange().GetClientAuth()
	if clientAuth.GetMethod() != api.OAuthClientAuth_PRIVATE_KEY_JWT {
		t.Fatalf("client auth method = %v, want PRIVATE_KEY_JWT", clientAuth.GetMethod())
	}
	if clientAuth.ClientSecret != nil {
		t.Fatalf("client secret = %q, want nil", clientAuth.GetClientSecret())
	}
	privateKeyJWT := clientAuth.GetPrivateKeyJwt()
	if privateKeyJWT == nil {
		t.Fatal("privateKeyJwt is nil, want configured settings")
	}
	if privateKeyJWT.GetSigningKey() == "" {
		t.Fatal("signing key is empty, want secret value")
	}
	if privateKeyJWT.GetAlg() != api.OAuthClientAuth_PrivateKeyJwt_ES256 {
		t.Fatalf("privateKeyJwt alg = %v, want ES256", privateKeyJWT.GetAlg())
	}
	if privateKeyJWT.GetKid() != "kid-1" {
		t.Fatalf("privateKeyJwt kid = %q, want kid-1", privateKeyJWT.GetKid())
	}
	if privateKeyJWT.GetAssertionAudience() != "https://issuer.example.com/oauth/token" {
		t.Fatalf("privateKeyJwt assertion audience = %q, want token endpoint URL", privateKeyJWT.GetAssertionAudience())
	}
}

func TestOAuthTokenExchangeRejectsUnsupportedConfigurations(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)
	tests := []struct {
		name string
		auth agentgateway.OAuthTokenExchange
		want string
	}{
		{
			name: "id-jag",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef:         oauthTokenEndpointRef(),
				RequestedTokenType: ptr.Of(agentgateway.OAuthTokenTypeIDJAG),
			},
			want: "IdJag is only supported by crossAppAccess",
		},
		{
			name: "jwt-bearer-actor-token",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef: oauthTokenEndpointRef(),
				GrantType:  ptr.Of(agentgateway.OAuthGrantTypeJwtBearer),
				ActorToken: &agentgateway.OAuthActorToken{
					Source: agentgateway.AuthorizationExtractionLocation{
						AuthorizationLocationFields: agentgateway.AuthorizationLocationFields{
							Header: &agentgateway.AuthorizationHeaderLocation{Name: "X-Actor-Token"},
						},
					},
				},
			},
			want: "actorToken is only valid with TokenExchange",
		},
		{
			name: "jwt-bearer-requested-token-type",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef:         oauthTokenEndpointRef(),
				GrantType:          ptr.Of(agentgateway.OAuthGrantTypeJwtBearer),
				RequestedTokenType: ptr.Of(agentgateway.OAuthTokenTypeAccessToken),
			},
			want: "requestedTokenType is only valid with TokenExchange",
		},
		{
			name: "may-act-without-jwt-actor",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef: oauthTokenEndpointRef(),
				ActorToken: &agentgateway.OAuthActorToken{
					Source: agentgateway.AuthorizationExtractionLocation{
						AuthorizationLocationFields: agentgateway.AuthorizationLocationFields{
							Header: &agentgateway.AuthorizationHeaderLocation{Name: "X-Actor-Token"},
						},
					},
					TokenType: ptr.Of(agentgateway.OAuthTokenTypeAccessToken),
					MayAct:    ptr.Of(agentgateway.OAuthMayActValidationModeRequired),
				},
			},
			want: "mayAct Required requires tokenType Jwt",
		},
		{
			name: "invalid-subject-source-cel",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef: oauthTokenEndpointRef(),
				SubjectToken: &agentgateway.OAuthTokenSpec{
					Source: &agentgateway.AuthorizationExtractionLocation{
						Expression: ptr.Of(agentgateway.CELExpression("((")),
					},
				},
			},
			want: "oauth subjectToken source expression is not a valid CEL expression",
		},
		{
			name: "invalid-actor-source-cel",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef: oauthTokenEndpointRef(),
				ActorToken: &agentgateway.OAuthActorToken{
					Source: agentgateway.AuthorizationExtractionLocation{
						Expression: ptr.Of(agentgateway.CELExpression("((")),
					},
				},
			},
			want: "oauth actorToken source expression is not a valid CEL expression",
		},
		{
			name: "reserved-additional-param",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef: oauthTokenEndpointRef(),
				AdditionalParams: map[string]agentgateway.CELExpression{
					"scope": "request.path",
				},
			},
			want: "overrides a reserved OAuth parameter",
		},
		{
			name: "private-key-jwt-without-method",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef: oauthTokenEndpointRef(),
				ClientAuth: &agentgateway.OAuthClientAuth{
					ClientID: "gateway",
					PrivateKeyJWT: &agentgateway.OAuthPrivateKeyJWT{
						SigningKeyRef: agentgateway.LocalSecretKeyRef{
							Name: "missing",
						},
						AssertionAudience: "https://issuer.example.com/oauth/token",
					},
				},
			},
			want: "privateKeyJwt requires method PrivateKeyJwt",
		},
		{
			name: "private-key-jwt-method-without-settings",
			auth: agentgateway.OAuthTokenExchange{
				BackendRef: oauthTokenEndpointRef(),
				ClientAuth: &agentgateway.OAuthClientAuth{
					ClientID: "gateway",
					Method:   ptr.Of(agentgateway.OAuthClientAuthMethodPrivateKeyJWT),
				},
			},
			want: "method PrivateKeyJwt requires privateKeyJwt settings",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, err := buildOAuthTokenExchangePolicy(ctx, &tt.auth, "default")
			if err == nil || !strings.Contains(err.Error(), tt.want) {
				t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want containing %q", err, tt.want)
			}
		})
	}
}

func TestTranslateBackendAuthPreservesInvalidOAuthPolicy(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)
	policy := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "oauth",
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					Auth: &agentgateway.BackendAuth{
						OAuthTokenExchange: &agentgateway.OAuthTokenExchange{
							BackendRef: oauthTokenEndpointRef(),
							SubjectToken: &agentgateway.OAuthTokenSpec{
								Source: &agentgateway.AuthorizationExtractionLocation{
									Expression: ptr.Of(agentgateway.CELExpression("((")),
								},
							},
						},
					},
				},
			},
		},
	}

	p, err := translateBackendAuth(ctx, policy, "default/oauth")
	if err == nil || !strings.Contains(err.Error(), "oauth subjectToken source expression is not a valid CEL expression") {
		t.Fatalf("translateBackendAuth() error = %v, want invalid CEL error", err)
	}
	if p.GetBackend().GetAuth().GetOauthTokenExchange() == nil {
		t.Fatalf("translateBackendAuth() policy = %v, want oauth token exchange auth", p)
	}
}

func TestOAuthTokenExchangeEnumDefaulting(t *testing.T) {
	ctx := oauthTestPolicyCtx(t, &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "oauth-client",
		},
		Data: map[string][]byte{
			"clientSecret": []byte("s3cr3t"),
		},
	})

	policy, err := buildOAuthTokenExchangePolicy(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
		ClientAuth: &agentgateway.OAuthClientAuth{
			ClientID: "gateway",
			SecretRef: &agentgateway.LocalSecretKeyRef{
				Name: "oauth-client",
			},
		},
	}, "default")
	if err != nil {
		t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want nil", err)
	}

	oauth := policy.GetOauthTokenExchange()
	if oauth.GetGrantType() != api.OAuthTokenExchange_UNSPECIFIED {
		t.Fatalf("grant type = %v, want UNSPECIFIED", oauth.GetGrantType())
	}
	if oauth.GetClientAuth().GetMethod() != api.OAuthClientAuth_UNSPECIFIED {
		t.Fatalf("client auth method = %v, want UNSPECIFIED", oauth.GetClientAuth().GetMethod())
	}
}

func TestOAuthTokenExchangeTokenTypeTranslation(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)

	path := "/oauth/token"
	policy, err := buildOAuthTokenExchangePolicy(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
		Path:       &path,
		SubjectToken: &agentgateway.OAuthTokenSpec{
			TokenType: ptr.Of(agentgateway.OAuthTokenTypeAccessToken),
		},
		ActorToken: &agentgateway.OAuthActorToken{
			Source: agentgateway.AuthorizationExtractionLocation{
				AuthorizationLocationFields: agentgateway.AuthorizationLocationFields{
					Header: &agentgateway.AuthorizationHeaderLocation{Name: "X-Actor-Token"},
				},
			},
			TokenType: ptr.Of(agentgateway.OAuthTokenTypeJWT),
			MayAct:    ptr.Of(agentgateway.OAuthMayActValidationModeRequired),
		},
		RequestedTokenType: ptr.Of(agentgateway.OAuthTokenTypeIDToken),
		Location: &agentgateway.AuthorizationLocation{
			AuthorizationLocationFields: agentgateway.AuthorizationLocationFields{
				Header: &agentgateway.AuthorizationHeaderLocation{Name: "X-Exchanged-Token"},
			},
		},
	}, "default")
	if err != nil {
		t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want nil", err)
	}

	oauth := policy.GetOauthTokenExchange()
	if oauth.GetTokenEndpointPath() != path {
		t.Fatalf("token endpoint path = %q, want %q", oauth.GetTokenEndpointPath(), path)
	}
	if oauth.GetSubjectToken().GetTokenType() != "urn:ietf:params:oauth:token-type:access_token" {
		t.Fatalf("subject token type = %q, want access_token URN", oauth.GetSubjectToken().GetTokenType())
	}
	if oauth.GetActorToken().GetTokenType() != "urn:ietf:params:oauth:token-type:jwt" {
		t.Fatalf("actor token type = %q, want jwt URN", oauth.GetActorToken().GetTokenType())
	}
	if !oauth.GetActorToken().GetEnforceMayAct() {
		t.Fatal("actor enforceMayAct = false, want true")
	}
	if oauth.GetRequestedTokenType() != "urn:ietf:params:oauth:token-type:id_token" {
		t.Fatalf("requested token type = %q, want id_token URN", oauth.GetRequestedTokenType())
	}
	if oauth.GetAuthorizationLocation().GetHeader().GetName() != "X-Exchanged-Token" {
		t.Fatalf("authorization location header = %q, want X-Exchanged-Token", oauth.GetAuthorizationLocation().GetHeader().GetName())
	}
}

func TestOAuthTokenExchangeCustomSubjectTokenTypeTranslation(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)

	path := "/oauth/token"
	customTokenType := agentgateway.OAuthTokenType("urn:company:domain:human")
	policy, err := buildOAuthTokenExchangePolicy(ctx, &agentgateway.OAuthTokenExchange{
		BackendRef: oauthTokenEndpointRef(),
		Path:       &path,
		SubjectToken: &agentgateway.OAuthTokenSpec{
			TokenType: new(customTokenType),
		},
	}, "default")
	if err != nil {
		t.Fatalf("buildOAuthTokenExchangePolicy() error = %v, want nil", err)
	}

	oauth := policy.GetOauthTokenExchange()
	if oauth.GetSubjectToken().GetTokenType() != string(customTokenType) {
		t.Fatalf("subject token type = %q, want %q", oauth.GetSubjectToken().GetTokenType(), customTokenType)
	}
}

func TestOAuthTokenExchangeRejectsInvalidCustomTokenTypes(t *testing.T) {
	tests := []struct {
		name      string
		buildAuth func(agentgateway.OAuthTokenType) *agentgateway.OAuthTokenExchange
		tokenType agentgateway.OAuthTokenType
		wantErr   string
	}{
		{
			name: "subject typo",
			buildAuth: func(tokenType agentgateway.OAuthTokenType) *agentgateway.OAuthTokenExchange {
				return &agentgateway.OAuthTokenExchange{
					BackendRef: oauthTokenEndpointRef(),
					SubjectToken: &agentgateway.OAuthTokenSpec{
						TokenType: new(tokenType),
					},
				}
			},
			tokenType: agentgateway.OAuthTokenType("JWt"),
			wantErr:   "oauth subjectToken tokenType",
		},
		{
			name: "subject fragment",
			buildAuth: func(tokenType agentgateway.OAuthTokenType) *agentgateway.OAuthTokenExchange {
				return &agentgateway.OAuthTokenExchange{
					BackendRef: oauthTokenEndpointRef(),
					SubjectToken: &agentgateway.OAuthTokenSpec{
						TokenType: new(tokenType),
					},
				}
			},
			tokenType: agentgateway.OAuthTokenType("https://tokens.example/custom#fragment"),
			wantErr:   "without a fragment",
		},
		{
			name: "actor typo",
			buildAuth: func(tokenType agentgateway.OAuthTokenType) *agentgateway.OAuthTokenExchange {
				return &agentgateway.OAuthTokenExchange{
					BackendRef: oauthTokenEndpointRef(),
					ActorToken: &agentgateway.OAuthActorToken{
						Source: agentgateway.AuthorizationExtractionLocation{
							AuthorizationLocationFields: agentgateway.AuthorizationLocationFields{
								Header: &agentgateway.AuthorizationHeaderLocation{Name: "X-Actor-Token"},
							},
						},
						TokenType: new(tokenType),
					},
				}
			},
			tokenType: agentgateway.OAuthTokenType("JWt"),
			wantErr:   "oauth actorToken tokenType",
		},
		{
			name: "actor fragment",
			buildAuth: func(tokenType agentgateway.OAuthTokenType) *agentgateway.OAuthTokenExchange {
				return &agentgateway.OAuthTokenExchange{
					BackendRef: oauthTokenEndpointRef(),
					ActorToken: &agentgateway.OAuthActorToken{
						Source: agentgateway.AuthorizationExtractionLocation{
							AuthorizationLocationFields: agentgateway.AuthorizationLocationFields{
								Header: &agentgateway.AuthorizationHeaderLocation{Name: "X-Actor-Token"},
							},
						},
						TokenType: new(tokenType),
					},
				}
			},
			tokenType: agentgateway.OAuthTokenType("https://tokens.example/custom#fragment"),
			wantErr:   "without a fragment",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ctx := oauthTestPolicyCtx(t)

			_, err := buildOAuthTokenExchangePolicy(ctx, tt.buildAuth(tt.tokenType), "default")
			if err == nil {
				t.Fatal("buildOAuthTokenExchangePolicy() error = nil, want invalid token type error")
			}
			if !strings.Contains(err.Error(), tt.wantErr) {
				t.Fatalf("buildOAuthTokenExchangePolicy() error = %q, want containing %q", err, tt.wantErr)
			}
		})
	}
}
