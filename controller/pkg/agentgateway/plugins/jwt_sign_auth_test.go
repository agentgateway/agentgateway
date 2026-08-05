package plugins

import (
	"errors"
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

type jwtSignErrorCredentialResolver struct{}

func (jwtSignErrorCredentialResolver) ResolveCredentialRef(
	krt.HandlerContext,
	agentgateway.LocalSecretObjectRef,
	string,
) (map[string][]byte, error) {
	return nil, errors.New("resolver-secret-data-marker")
}

func jwtSignTestPolicy(auth *agentgateway.JwtSignAuth) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default"},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					Auth: &agentgateway.BackendAuth{JwtSign: auth},
				},
			},
		},
	}
}

func translatedJwtSign(t *testing.T, policy *api.Policy) *api.JwtSign {
	t.Helper()
	if policy == nil {
		t.Fatal("translated policy is nil")
	}
	jwtSign := policy.GetBackend().GetAuth().GetJwtSign()
	if jwtSign == nil {
		t.Fatal("translated policy kind is not jwtSign")
	}
	return jwtSign
}

func jwtSignSecret(data map[string][]byte) *corev1.Secret {
	return &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "jwt-sign-secret"},
		Data:       data,
	}
}

func TestJwtSignNilSigningAlgDefaultsToUnspecified(t *testing.T) {
	got := translateJwtSignSigningAlg(nil)
	if got != api.JwtSigningAlg_JWT_SIGNING_ALG_UNSPECIFIED {
		t.Fatalf("translateJwtSignSigningAlg(nil) = %v, want JWT_SIGNING_ALG_UNSPECIFIED", got)
	}
}

func TestJwtSignPreservesUnsupportedSigningAlgForDataPlaneRejection(t *testing.T) {
	ctx := oauthTestPolicyCtx(t, jwtSignSecret(map[string][]byte{
		"signingKey": []byte("pem-value"),
	}))
	badAlg := agentgateway.JwtSigningAlg("HS256")
	policy := jwtSignTestPolicy(&agentgateway.JwtSignAuth{
		SigningKeyRef: agentgateway.LocalSecretObjectRef{Name: "jwt-sign-secret"},
		Alg:           &badAlg,
		Claims:        map[string]apiextensionsv1.JSON{"iss": {Raw: []byte(`"acct.user"`)}},
	})

	p, err := translateBackendAuth(ctx, policy, "default/jwt-sign")
	if err != nil {
		t.Fatalf("translateBackendAuth() error = %v, want nil", err)
	}
	jwtSign := translatedJwtSign(t, p)
	if jwtSign.TranslationError != nil {
		t.Fatalf("translationError = %q, want nil", jwtSign.GetTranslationError())
	}
	if jwtSign.GetAlg() != api.JwtSigningAlg(-1) {
		t.Fatalf("jwtSign alg = %v, want unknown value for data-plane rejection", jwtSign.GetAlg())
	}
}

// CEL admission validation cannot inspect map[string]JSON fields, so the
// controller must reject signer-reserved claims during translation instead.
func TestJwtSignRejectsReservedClaims(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)
	for _, reserved := range []string{"iat", "exp", "nbf"} {
		policy := jwtSignTestPolicy(&agentgateway.JwtSignAuth{
			SigningKeyRef: agentgateway.LocalSecretObjectRef{Name: "jwt-sign-secret"},
			Claims: map[string]apiextensionsv1.JSON{
				"iss":    {Raw: []byte(`"acct.user"`)},
				reserved: {Raw: []byte(`123`)},
			},
		})

		p, err := translateBackendAuth(ctx, policy, "default/jwt-sign")
		if err == nil || !strings.Contains(err.Error(), "reserved for the signer") {
			t.Fatalf("translateBackendAuth() error = %v, want reserved claim error for %q", err, reserved)
		}
		jwtSign := translatedJwtSign(t, p)
		if jwtSign.TranslationError == nil || !strings.Contains(jwtSign.GetTranslationError(), reserved) {
			t.Fatalf("translationError = %q, want reserved claim %q", jwtSign.GetTranslationError(), reserved)
		}
	}
}

func TestJwtSignAllowsNoStaticClaims(t *testing.T) {
	ctx := oauthTestPolicyCtx(t, jwtSignSecret(map[string][]byte{
		"signingKey": []byte("pem-value"),
	}))
	policy := jwtSignTestPolicy(&agentgateway.JwtSignAuth{
		SigningKeyRef: agentgateway.LocalSecretObjectRef{Name: "jwt-sign-secret"},
	})

	p, err := translateBackendAuth(ctx, policy, "default/jwt-sign")
	if err != nil {
		t.Fatalf("translateBackendAuth() error = %v, want nil", err)
	}
	if claims := translatedJwtSign(t, p).GetClaims(); len(claims) != 0 {
		t.Fatalf("translated claims = %v, want empty", claims)
	}
}

func TestJwtSignTranslationErrorSanitizesResolverError(t *testing.T) {
	ctx := oauthTestPolicyCtx(t)
	ctx.CredentialResolver = jwtSignErrorCredentialResolver{}
	policy := jwtSignTestPolicy(&agentgateway.JwtSignAuth{
		SigningKeyRef: agentgateway.LocalSecretObjectRef{Name: "jwt-sign-secret"},
		Claims:        map[string]apiextensionsv1.JSON{"iss": {Raw: []byte(`"acct.user"`)}},
	})

	p, err := translateBackendAuth(ctx, policy, "default/jwt-sign")
	want := "failed to resolve jwtSign signing secret default/jwt-sign-secret"
	if err == nil || !strings.Contains(err.Error(), want) {
		t.Fatalf("translateBackendAuth() error = %v, want %q", err, want)
	}
	jwtSign := translatedJwtSign(t, p)
	if jwtSign.TranslationError == nil || !strings.Contains(jwtSign.GetTranslationError(), want) {
		t.Fatalf("translationError = %q, want %q", jwtSign.GetTranslationError(), want)
	}
	if jwtSign.GetSigningKey() != "" {
		t.Fatal("error-bearing jwtSign must not contain a signing key")
	}
	if strings.Contains(err.Error(), "resolver-secret-data-marker") ||
		strings.Contains(jwtSign.GetTranslationError(), "resolver-secret-data-marker") {
		t.Fatal("translation diagnostic leaked resolver error details")
	}
}

func TestJwtSignInvalidClaimDoesNotLeakValue(t *testing.T) {
	const claimValueMarker = "claim-value-secret-marker"
	ctx := oauthTestPolicyCtx(t, jwtSignSecret(map[string][]byte{
		"signingKey": []byte("pem-value"),
	}))
	policy := jwtSignTestPolicy(&agentgateway.JwtSignAuth{
		SigningKeyRef: agentgateway.LocalSecretObjectRef{Name: "jwt-sign-secret"},
		Claims: map[string]apiextensionsv1.JSON{
			"password": {Raw: []byte(claimValueMarker)},
		},
	})

	p, err := translateBackendAuth(ctx, policy, "default/jwt-sign")
	if err == nil || !strings.Contains(err.Error(), `jwtSign claim "password" contains invalid JSON`) {
		t.Fatalf("translateBackendAuth() error = %v, want sanitized invalid claim error", err)
	}
	jwtSign := translatedJwtSign(t, p)
	if strings.Contains(err.Error(), claimValueMarker) ||
		strings.Contains(jwtSign.GetTranslationError(), claimValueMarker) {
		t.Fatalf("diagnostic leaked claim value marker %q", claimValueMarker)
	}
}

func TestJwtSignInvalidClaimErrorsAreDeterministic(t *testing.T) {
	values := map[string]apiextensionsv1.JSON{
		"z-last":  {Raw: []byte("invalid-z")},
		"a-first": {Raw: []byte("invalid-a")},
	}
	const want = "jwtSign claim \"a-first\" contains invalid JSON\njwtSign claim \"z-last\" contains invalid JSON"

	for range 10 {
		_, err := translateJSONValueMap("jwtSign claim", values)
		if err == nil || err.Error() != want {
			t.Fatalf("translateJSONValueMap() error = %q, want %q", err, want)
		}
	}
}
