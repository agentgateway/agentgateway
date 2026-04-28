package oidc

import (
	"crypto/tls"
	"strings"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestCollapseOidcSourcesUsesLowestTTL(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/.well-known/openid-configuration"}
	requestKey := testCollectionOidcRequestKey(target)
	shared := CollapseOidcSources(krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key: requestKey,
		Objects: []OidcSource{
			{
				OwnerKey:       OidcOwnerID{Name: "one"},
				RequestKey:     requestKey,
				ExpectedIssuer: "https://issuer.example",
				Target:         target,
				TTL:            5 * time.Minute,
			},
			{
				OwnerKey:       OidcOwnerID{Name: "two"},
				RequestKey:     requestKey,
				ExpectedIssuer: "https://issuer.example",
				Target:         target,
				TTL:            2 * time.Minute,
			},
		},
	})

	if assert.NotNil(t, shared) {
		assert.Equal(t, 2*time.Minute, shared.TTL)
	}
}

func TestCollapseOidcSourcesReturnsNilForEmptyGroup(t *testing.T) {
	shared := CollapseOidcSources(krt.IndexObject[remotehttp.FetchKey, OidcSource]{})

	assert.Nil(t, shared)
}

func TestCollapseOidcSourcesUsesSortedOwnerForTargetAndTLSConfig(t *testing.T) {
	earlierTarget := remotehttp.FetchTarget{URL: "https://issuer-a.example/.well-known/openid-configuration"}
	laterTarget := remotehttp.FetchTarget{URL: "https://issuer-b.example/.well-known/openid-configuration"}
	requestKey := testCollectionOidcRequestKey(earlierTarget)
	earlierTLS := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: "issuer-a.example"}
	laterTLS := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: "issuer-b.example"}

	shared := CollapseOidcSources(krt.IndexObject[remotehttp.FetchKey, OidcSource]{
		Key: requestKey,
		Objects: []OidcSource{
			{
				OwnerKey:       OidcOwnerID{Name: "z-owner"},
				RequestKey:     requestKey,
				ExpectedIssuer: "https://issuer-a.example",
				Target:         laterTarget,
				TLSConfig:      laterTLS,
				TTL:            5 * time.Minute,
			},
			{
				OwnerKey:       OidcOwnerID{Name: "a-owner"},
				RequestKey:     requestKey,
				ExpectedIssuer: "https://issuer-a.example",
				Target:         earlierTarget,
				TLSConfig:      earlierTLS,
				TTL:            10 * time.Minute,
			},
		},
	})

	if assert.NotNil(t, shared) {
		assert.Equal(t, earlierTarget, shared.Target)
		assert.Same(t, earlierTLS, shared.TLSConfig)
	}
}

func testCollectionOidcRequestKey(target remotehttp.FetchTarget) remotehttp.FetchKey {
	expectedIssuer := strings.TrimSuffix(target.URL, "/.well-known/openid-configuration")
	return oidcRequestKey(target, expectedIssuer)
}
