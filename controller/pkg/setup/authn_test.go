package setup

import (
	"net/http"
	"testing"

	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/kubernetes/fake"

	"istio.io/istio/pkg/security"
)

func TestAuthenticateUsesLegacyAndCurrentXDSTokenAudiences(t *testing.T) {
	originalValidateK8sJWT := validateK8sJWT
	t.Cleanup(func() {
		validateK8sJWT = originalValidateK8sJWT
	})

	validateK8sJWT = func(_ kubernetes.Interface, targetToken string, audiences []string) (security.KubernetesInfo, error) {
		if targetToken != "test-token" {
			t.Fatalf("unexpected token %q", targetToken)
		}
		if len(audiences) != 2 {
			t.Fatalf("expected 2 audiences, got %d", len(audiences))
		}
		if !containsAudience(audiences, "kgateway") || !containsAudience(audiences, "agentgateway") {
			t.Fatalf("unexpected audiences: %v", audiences)
		}

		return security.KubernetesInfo{
			PodNamespace:      "default",
			PodServiceAccount: "agentgateway",
		}, nil
	}

	authenticator := NewKubeJWTAuthenticator(fake.NewSimpleClientset())

	caller, err := authenticator.authenticate("test-token")
	if err != nil {
		t.Fatalf("authenticate returned error: %v", err)
	}
	if caller == nil {
		t.Fatal("expected caller, got nil")
	}
	if caller.KubernetesInfo.PodNamespace != "default" {
		t.Fatalf("unexpected namespace %q", caller.KubernetesInfo.PodNamespace)
	}
	if caller.KubernetesInfo.PodServiceAccount != "agentgateway" {
		t.Fatalf("unexpected service account %q", caller.KubernetesInfo.PodServiceAccount)
	}
}

func TestExtractRequestToken(t *testing.T) {
	t.Run("extracts bearer token", func(t *testing.T) {
		req := testRequestWithAuthHeader("Bearer test-token")

		token, err := extractRequestToken(req)
		if err != nil {
			t.Fatalf("extractRequestToken returned error: %v", err)
		}
		if token != "test-token" {
			t.Fatalf("unexpected token %q", token)
		}
	})

	t.Run("rejects missing authorization header", func(t *testing.T) {
		req := testRequestWithAuthHeader("")

		_, err := extractRequestToken(req)
		if err == nil {
			t.Fatal("expected error for missing authorization header")
		}
	})

	t.Run("rejects non bearer authorization header", func(t *testing.T) {
		req := testRequestWithAuthHeader("Basic abc123")

		_, err := extractRequestToken(req)
		if err == nil {
			t.Fatal("expected error for non-bearer authorization header")
		}
	})
}

func containsAudience(audiences []string, want string) bool {
	for _, audience := range audiences {
		if audience == want {
			return true
		}
	}
	return false
}

func testRequestWithAuthHeader(authHeader string) *http.Request {
	req, err := http.NewRequest(http.MethodGet, "http://example.com", nil)
	if err != nil {
		panic(err)
	}
	if authHeader != "" {
		req.Header.Set(authorizationHeader, authHeader)
	}
	return req
}
