package jwks

import (
	"crypto/tls"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestCollapseJwksSourcesUsesLowestTTL(t *testing.T) {
	target := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}
	shared := collapseJwksSources(krt.IndexObject[remotehttp.FetchKey, JwksSource]{
		Key: target.Key(),
		Objects: []JwksSource{
			{
				OwnerKey:   JwksOwnerID{Name: "one"},
				RequestKey: target.Key(),
				Target:     target,
				TTL:        5 * time.Minute,
			},
			{
				OwnerKey:   JwksOwnerID{Name: "two"},
				RequestKey: target.Key(),
				Target:     target,
				TTL:        2 * time.Minute,
			},
		},
	})

	if assert.NotNil(t, shared) {
		assert.Equal(t, 2*time.Minute, shared.TTL)
	}
}

func TestCollapseJwksSourcesReturnsNilForEmptyGroup(t *testing.T) {
	shared := collapseJwksSources(krt.IndexObject[remotehttp.FetchKey, JwksSource]{})

	assert.Nil(t, shared)
}

func TestCollapseJwksSourcesUsesSortedOwnerForTargetAndTLSConfig(t *testing.T) {
	requestKey := remotehttp.FetchTarget{URL: "https://issuer.example/jwks"}.Key()
	earlierTarget := remotehttp.FetchTarget{URL: "https://issuer-a.example/jwks"}
	laterTarget := remotehttp.FetchTarget{URL: "https://issuer-b.example/jwks"}
	earlierTLS := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: "issuer-a.example"}
	laterTLS := &tls.Config{MinVersion: tls.VersionTLS12, ServerName: "issuer-b.example"}

	shared := collapseJwksSources(krt.IndexObject[remotehttp.FetchKey, JwksSource]{
		Key: requestKey,
		Objects: []JwksSource{
			{
				OwnerKey:   JwksOwnerID{Name: "z-owner"},
				RequestKey: requestKey,
				Target:     laterTarget,
				TLSConfig:  laterTLS,
				TTL:        5 * time.Minute,
			},
			{
				OwnerKey:   JwksOwnerID{Name: "a-owner"},
				RequestKey: requestKey,
				Target:     earlierTarget,
				TLSConfig:  earlierTLS,
				TTL:        10 * time.Minute,
			},
		},
	})

	if assert.NotNil(t, shared) {
		assert.Equal(t, earlierTarget, shared.Target)
		assert.Same(t, earlierTLS, shared.TLSConfig)
	}
}
