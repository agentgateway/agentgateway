package controller_test

import (
	"context"
	"testing"

	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/controller"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func TestNewControllerBuilderErrorsWhenJWKSLookupIsNil(t *testing.T) {
	builder, err := controller.NewControllerBuilder(context.Background(), controller.StartConfig{})
	if err == nil {
		t.Fatal("expected jwks lookup error")
	}
	if builder != nil {
		t.Fatal("expected nil builder")
	}
	if err.Error() != "jwks lookup is not configured" {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestPluginsRegistersJWKSAwareBuiltins(t *testing.T) {
	collections := testutils.BuildMockCollection(t, nil)
	resolver := testutils.BuildRemoteHTTPResolver(collections)
	jwksLookup := testutils.BuildJWKSLookup(collections)
	oidcLookup := testutils.BuildOIDCLookup(collections)

	plug := plugins.MergePlugins(controller.Plugins(collections, resolver, jwksLookup, oidcLookup)...)

	if got := len(controller.Plugins(collections, resolver, jwksLookup, oidcLookup)); got != 5 {
		t.Fatalf("expected 5 built-in plugins, got %d", got)
	}
	if _, ok := plug.ContributesPolicies[wellknown.AgentgatewayPolicyGVK.GroupKind()]; !ok {
		t.Fatalf("expected %v policy contribution", wellknown.AgentgatewayPolicyGVK.GroupKind())
	}
	if _, ok := plug.ContributesBackends[wellknown.AgentgatewayBackendGVK.GroupKind()]; !ok {
		t.Fatalf("expected %v backend contribution", wellknown.AgentgatewayBackendGVK.GroupKind())
	}
}

func TestPluginsPreserveExtraContributionWhenMerged(t *testing.T) {
	collections := testutils.BuildMockCollection(t, nil)
	resolver := testutils.BuildRemoteHTTPResolver(collections)
	jwksLookup := testutils.BuildJWKSLookup(collections)
	oidcLookup := testutils.BuildOIDCLookup(collections)
	extraGK := schema.GroupKind{Group: "test.agentgateway.dev", Kind: "ExtraPolicy"}

	plug := plugins.MergePlugins(append(controller.Plugins(collections, resolver, jwksLookup, oidcLookup), plugins.AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]plugins.PolicyPlugin{
			extraGK: {},
		},
	})...)

	if _, ok := plug.ContributesPolicies[wellknown.AgentgatewayPolicyGVK.GroupKind()]; !ok {
		t.Fatalf("expected built-in policy contribution to be preserved")
	}
	if _, ok := plug.ContributesBackends[wellknown.AgentgatewayBackendGVK.GroupKind()]; !ok {
		t.Fatalf("expected built-in backend contribution to be preserved")
	}
	if _, ok := plug.ContributesPolicies[extraGK]; !ok {
		t.Fatalf("expected extra policy contribution %v to be preserved", extraGK)
	}
}
