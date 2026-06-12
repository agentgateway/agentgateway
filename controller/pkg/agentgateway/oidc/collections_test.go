package oidc

import (
	"crypto/tls"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube/krt"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

func TestOwnerFromPolicyExtractsOidcOwner(t *testing.T) {
	policy := testOidcPolicy("p1")

	owner, ok := OwnerFromPolicy(policy)

	require.True(t, ok)
	require.Equal(t, "p1", owner.ID.Name)
	require.Equal(t, "default", owner.ID.Namespace)
	require.Equal(t, "spec.traffic.oidc", owner.ID.Path)
	require.Equal(t, OidcRefreshInterval, owner.TTL)
}

func TestOwnerFromPolicyReturnsNoneWhenOidcAbsent(t *testing.T) {
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
					TargetRefs: make([]agentgateway.LocalPolicyTargetReferenceWithSectionName, 1),
				},
			},
		},
		{
			name: "traffic block without oidc",
			policy: &agentgateway.AgentgatewayPolicy{
				ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "p"},
				Spec: agentgateway.AgentgatewayPolicySpec{
					TargetRefs: make([]agentgateway.LocalPolicyTargetReferenceWithSectionName, 1),
					Traffic:    &agentgateway.Traffic{},
				},
			},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			_, ok := OwnerFromPolicy(tc.policy)
			require.False(t, ok)
		})
	}
}

func TestCollapseOidcSourcesPicksMinTTL(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}
	requestKey := oidcRequestKey(target, "https://idp.example", false)
	grouped := krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key: requestKey,
		Objects: []OidcSource{
			{
				OwnerKey: remotecache.OwnerID{Kind: remotecache.OwnerKindPolicy, Namespace: "default", Name: "long", Path: "spec.traffic.oidc"},
				oidcRequestSpec: oidcRequestSpec{
					RequestKey:     requestKey,
					ExpectedIssuer: "https://idp.example",
					Target:         target,
					TTL:            30 * time.Minute,
				},
			},
			{
				OwnerKey: remotecache.OwnerID{Kind: remotecache.OwnerKindPolicy, Namespace: "default", Name: "short", Path: "spec.traffic.oidc"},
				oidcRequestSpec: oidcRequestSpec{
					RequestKey:     requestKey,
					ExpectedIssuer: "https://idp.example",
					Target:         target,
					TTL:            5 * time.Minute,
				},
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
	requestKey := oidcRequestKey(target, "https://idp.example", false)
	source := func(name string, ttl time.Duration) OidcSource {
		return OidcSource{
			OwnerKey: remotecache.OwnerID{Kind: remotecache.OwnerKindPolicy, Namespace: "default", Name: name, Path: "spec.traffic.oidc"},
			oidcRequestSpec: oidcRequestSpec{
				RequestKey:     requestKey,
				ExpectedIssuer: "https://idp.example",
				Target:         target,
				TTL:            ttl,
			},
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

func TestCollapseOidcSourcesUsesSortedOwnerForTargetAndTLSConfig(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://idp.example/.well-known/openid-configuration"}.Key()
	earlierTarget := remotehttp.FetchTarget{URL: "https://idp-a.example/.well-known/openid-configuration"}
	laterTarget := remotehttp.FetchTarget{URL: "https://idp-b.example/.well-known/openid-configuration"}
	earlierTLS := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: "idp-a.example"}
	laterTLS := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: "idp-b.example"}

	shared := collapseOidcSources(krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key: requestKey,
		Objects: []OidcSource{
			{
				OwnerKey: remotecache.OwnerID{Name: "z-owner"},
				oidcRequestSpec: oidcRequestSpec{
					RequestKey: requestKey,
					Target:     laterTarget,
					TLSConfig:  laterTLS,
					TTL:        5 * time.Minute,
				},
			},
			{
				OwnerKey: remotecache.OwnerID{Name: "a-owner"},
				oidcRequestSpec: oidcRequestSpec{
					RequestKey: requestKey,
					Target:     earlierTarget,
					TLSConfig:  earlierTLS,
					TTL:        10 * time.Minute,
				},
			},
		},
	})

	require.NotNil(t, shared)
	require.Equal(t, earlierTarget, shared.Target)
	require.Same(t, earlierTLS, shared.TLSConfig)
}

func TestNewCollectionsCollapsesSharedKeyAcrossPolicies(t *testing.T) {
	krtOpts := krtutil.NewKrtOptions(t.Context().Done(), new(krt.DebugHandler))
	policies := krt.NewStaticCollection(nil, []*agentgateway.AgentgatewayPolicy{
		testOidcPolicy("a"),
		testOidcPolicy("b"),
	}, krtOpts.ToOptions("OidcPolicies")...)

	collections := NewCollections(CollectionInputs{
		AgentgatewayPolicies: policies,
		Resolver:             NewResolver(nil),
		KrtOpts:              krtOpts,
	})

	requests := await(t, collections.SharedRequests, 1)
	require.Equal(t, OidcRefreshInterval, requests[0].TTL)
}

const testOidcIssuer = "https://idp.example"

func testOidcPolicy(name string) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      name,
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			TargetRefs: make([]agentgateway.LocalPolicyTargetReferenceWithSectionName, 1),
			Traffic: &agentgateway.Traffic{
				OIDC: &agentgateway.OIDC{
					IssuerURL:   testOidcIssuer,
					ClientID:    "agw",
					RedirectURI: "https://gateway.example/oauth2/callback",
				},
			},
		},
	}
}
