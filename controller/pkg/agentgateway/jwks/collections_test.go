package jwks

import (
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
