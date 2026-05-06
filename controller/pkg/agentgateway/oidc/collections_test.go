package oidc

import (
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil/krttest"
)

func TestOwnersFromPolicyExtractsOidcOwner(t *testing.T) {
	policy := testOidcPolicy("p1", 30*time.Minute)

	owners := OwnersFromPolicy(policy)

	require.Len(t, owners, 1)
	require.Equal(t, "p1", owners[0].ID.Name)
	require.Equal(t, "default", owners[0].ID.Namespace)
	require.Equal(t, "spec.traffic.oidc", owners[0].ID.Path)
	require.Equal(t, 30*time.Minute, owners[0].TTL)
}

func TestOwnersFromPolicyReturnsNilWhenOidcAbsent(t *testing.T) {
	tests := []struct {
		name   string
		policy *agentgateway.AgentgatewayPolicy
	}{
		{
			name: "no targetRefs or selectors",
			policy: &agentgateway.AgentgatewayPolicy{
				ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "p"},
				Spec: agentgateway.AgentgatewayPolicySpec{
					Traffic: &agentgateway.Traffic{OIDC: &agentgateway.OIDC{}},
				},
			},
		},
		{
			name: "no traffic block",
			policy: &agentgateway.AgentgatewayPolicy{
				ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "p"},
				Spec: agentgateway.AgentgatewayPolicySpec{
					TargetRefs: make([]shared.LocalPolicyTargetReferenceWithSectionName, 1),
				},
			},
		},
		{
			name: "traffic block without oidc",
			policy: &agentgateway.AgentgatewayPolicy{
				ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "p"},
				Spec: agentgateway.AgentgatewayPolicySpec{
					TargetRefs: make([]shared.LocalPolicyTargetReferenceWithSectionName, 1),
					Traffic:    &agentgateway.Traffic{},
				},
			},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			require.Nil(t, OwnersFromPolicy(tc.policy))
		})
	}
}

func TestTTLForOIDCDefaultsToOneHour(t *testing.T) {
	cfg := agentgateway.OIDC{IssuerURL: "https://idp.example"}

	require.Equal(t, time.Hour, TTLForOIDC(cfg))
}

func TestTTLForOIDCRespectsRefreshInterval(t *testing.T) {
	cfg := agentgateway.OIDC{
		IssuerURL:       "https://idp.example",
		RefreshInterval: &metav1.Duration{Duration: 10 * time.Minute},
	}

	require.Equal(t, 10*time.Minute, TTLForOIDC(cfg))
}

func TestCollapseOidcSourcesPicksMinTTL(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, "https://idp.example")
	grouped := krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key: requestKey,
		Objects: []OidcSource{
			{
				OwnerKey:       OidcOwnerID{Kind: remotecache.OwnerKindPolicy, Namespace: "default", Name: "long", Path: "spec.traffic.oidc"},
				RequestKey:     requestKey,
				ExpectedIssuer: "https://idp.example",
				Target:         target,
				TTL:            30 * time.Minute,
			},
			{
				OwnerKey:       OidcOwnerID{Kind: remotecache.OwnerKindPolicy, Namespace: "default", Name: "short", Path: "spec.traffic.oidc"},
				RequestKey:     requestKey,
				ExpectedIssuer: "https://idp.example",
				Target:         target,
				TTL:            5 * time.Minute,
			},
		},
	}

	shared := collapseOidcSources(grouped)

	require.NotNil(t, shared)
	require.Equal(t, 5*time.Minute, shared.TTL)
	require.Equal(t, requestKey, shared.RequestKey)
}

func TestCollapseOidcSourcesIsDeterministicAcrossOwnerOrder(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, "https://idp.example")
	source := func(name string, ttl time.Duration) OidcSource {
		return OidcSource{
			OwnerKey:       OidcOwnerID{Kind: remotecache.OwnerKindPolicy, Namespace: "default", Name: name, Path: "spec.traffic.oidc"},
			RequestKey:     requestKey,
			ExpectedIssuer: "https://idp.example",
			Target:         target,
			TTL:            ttl,
		}
	}

	forwardOrder := collapseOidcSources(krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key:     requestKey,
		Objects: []OidcSource{source("a", 5*time.Minute), source("b", 10*time.Minute)},
	})
	reverseOrder := collapseOidcSources(krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key:     requestKey,
		Objects: []OidcSource{source("b", 10*time.Minute), source("a", 5*time.Minute)},
	})

	require.Equal(t, forwardOrder, reverseOrder,
		"shared request must not depend on owner order in the input slice")
}

func TestCollapseOidcSourcesEmptyReturnsNil(t *testing.T) {
	require.Nil(t, collapseOidcSources(krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key:     "any",
		Objects: nil,
	}))
}

func TestNewCollectionsCollapsesSharedKeyAcrossPolicies(t *testing.T) {
	krtOpts := krttest.KrtOptions(t)
	policies := krttest.NewStaticCollection(t, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("a", 30*time.Minute),
		testOidcPolicy("b", 10*time.Minute),
	}, krtOpts, "OidcPolicies")

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		KrtOpts:              krtOpts,
	})

	requests := krttest.Await(t, collections.SharedRequests, 1)
	require.Equal(t, 10*time.Minute, requests[0].TTL,
		"shared request must inherit the smallest TTL across owners")
}

const testOidcIssuer = "https://idp.example"

func testOidcPolicy(name string, refresh time.Duration) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: make([]shared.LocalPolicyTargetReferenceWithSectionName, 1),
			Traffic: &agentgateway.Traffic{
				OIDC: &agentgateway.OIDC{
					IssuerURL:       testOidcIssuer,
					ClientID:        "agw",
					RedirectURI:     "https://gateway.example/oauth2/callback",
					RefreshInterval: &metav1.Duration{Duration: refresh},
				},
			},
		},
	}
}
