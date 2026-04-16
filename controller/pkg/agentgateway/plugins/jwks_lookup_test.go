package plugins

import (
	"testing"

	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks"
)

func TestResolveJWKSInlineForOwnerErrorsWhenJWKSLookupIsNil(t *testing.T) {
	_, err := resolveJWKSInlineForOwner(PolicyCtx{
		Krt: krt.TestingDummyContext{},
	}, jwks.RemoteJwksOwner{})

	if err == nil {
		t.Fatal("expected jwks lookup error")
	}
	if err.Error() != "jwks lookup is not configured" {
		t.Fatalf("unexpected error: %v", err)
	}
}
