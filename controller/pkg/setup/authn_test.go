package setup

import (
	"net/http"
	"slices"
	"testing"

	"istio.io/istio/pkg/security"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/kubernetes/fake"
)

func TestAuthenticateUsesLegacyAndCurrentXDSTokenAudiences(t *testing.T) {
	authenticator := NewKubeJWTAuthenticator(fake.NewSimpleClientset())
	authenticator.validateJWT = func(_ kubernetes.Interface, targetToken string, audiences []string) (security.KubernetesInfo, error) {
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
		req := testRequestWithAuthHeader(t, "Bearer test-token")

		token, err := extractRequestToken(req)
		if err != nil {
			t.Fatalf("extractRequestToken returned error: %v", err)
		}
		if token != "test-token" {
			t.Fatalf("unexpected token %q", token)
		}
	})

	t.Run("rejects missing authorization header", func(t *testing.T) {
		req := testRequestWithAuthHeader(t, "")

		_, err := extractRequestToken(req)
		if err == nil {
			t.Fatal("expected error for missing authorization header")
		}
	})

	t.Run("rejects non bearer authorization header", func(t *testing.T) {
		req := testRequestWithAuthHeader(t, "Basic abc123")

		_, err := extractRequestToken(req)
		if err == nil {
			t.Fatal("expected error for non-bearer authorization header")
		}
	})
}

func containsAudience(audiences []string, want string) bool {
	return slices.Contains(audiences, want)
}

func testRequestWithAuthHeader(t *testing.T, authHeader string) *http.Request {
	t.Helper()

	req, err := http.NewRequest(http.MethodGet, "http://example.com", nil)
	if err != nil {
		t.Fatalf("http.NewRequest returned error: %v", err)
	}
	if authHeader != "" {
		req.Header.Set(authorizationHeader, authHeader)
	}
	return req
}
