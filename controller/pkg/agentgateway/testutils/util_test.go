package testutils

import "testing"

func TestBuildMockPolicyContextInjectsResolverAndJWKSLookup(t *testing.T) {
	ctx := BuildMockPolicyContext(t, nil)

	if ctx.Collections == nil {
		t.Fatal("expected collections to be populated")
	}
	if ctx.Resolver == nil {
		t.Fatal("expected resolver to be injected")
	}
	if ctx.JWKSLookup == nil {
		t.Fatal("expected jwks lookup to be injected")
	}
}
