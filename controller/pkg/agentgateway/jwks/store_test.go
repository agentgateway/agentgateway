package jwks

import (
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
)

func TestStoreKeepsSharedRequestAliveUntilLastOwnerIsRemoved(t *testing.T) {
	store := &Store{
		storePrefix:         DefaultJwksStorePrefix,
		deploymentNamespace: "agentgateway-system",
		sourcesByOwner:      make(map[OwnerKey]JwksSource),
		ownersByRequestKey:  make(map[RequestKey]map[OwnerKey]JwksSource),
	}

	req := Request{URL: "https://issuer.example/jwks"}
	key := req.Key()

	first := JwksSource{
		OwnerKey:   testOwner("one"),
		RequestKey: key,
		Request:    req,
		TTL:        10 * time.Minute,
	}
	second := JwksSource{
		OwnerKey:   testOwner("two"),
		RequestKey: key,
		Request:    req,
		TTL:        5 * time.Minute,
	}

	store.applyOwnerUpdate(first)
	update := store.applyOwnerUpdate(second)
	if assert.Len(t, update.actions, 1) {
		assert.False(t, update.actions[0].delete)
		if assert.NotNil(t, update.actions[0].upsert) {
			assert.Equal(t, 5*time.Minute, update.actions[0].upsert.TTL)
		}
	}

	update = store.removeOwner(first.OwnerKey)
	if assert.Len(t, update.actions, 1) {
		assert.False(t, update.actions[0].delete)
		if assert.NotNil(t, update.actions[0].upsert) {
			assert.Equal(t, key, update.actions[0].upsert.RequestKey)
			assert.Equal(t, second.TTL, update.actions[0].upsert.TTL)
		}
	}

	update = store.removeOwner(second.OwnerKey)
	if assert.Len(t, update.actions, 1) {
		assert.True(t, update.actions[0].delete)
		assert.Nil(t, update.actions[0].upsert)
		assert.Equal(t, key, update.actions[0].requestKey)
	}
}

func TestStoreMovingOwnerRemovesOrphanedPreviousRequest(t *testing.T) {
	store := &Store{
		storePrefix:         DefaultJwksStorePrefix,
		deploymentNamespace: "agentgateway-system",
		sourcesByOwner:      make(map[OwnerKey]JwksSource),
		ownersByRequestKey:  make(map[RequestKey]map[OwnerKey]JwksSource),
	}

	owner := testOwner("one")
	requestA := Request{URL: "https://issuer.example/a"}
	requestB := Request{URL: "https://issuer.example/b"}
	sourceA := JwksSource{OwnerKey: owner, RequestKey: requestA.Key(), Request: requestA, TTL: 10 * time.Minute}
	sourceB := JwksSource{OwnerKey: owner, RequestKey: requestB.Key(), Request: requestB, TTL: 15 * time.Minute}

	store.applyOwnerUpdate(sourceA)
	update := store.applyOwnerUpdate(sourceB)

	if assert.Len(t, update.actions, 2) {
		assert.True(t, update.actions[0].delete)
		assert.Equal(t, sourceA.RequestKey, update.actions[0].requestKey)
		assert.False(t, update.actions[1].delete)
		if assert.NotNil(t, update.actions[1].upsert) {
			assert.Equal(t, sourceB.RequestKey, update.actions[1].upsert.RequestKey)
			assert.Equal(t, sourceB.TTL, update.actions[1].upsert.TTL)
		}
	}
}

func TestStoreMovingOwnerRecomputesPreviousSharedRequest(t *testing.T) {
	store := &Store{
		storePrefix:         DefaultJwksStorePrefix,
		deploymentNamespace: "agentgateway-system",
		sourcesByOwner:      make(map[OwnerKey]JwksSource),
		ownersByRequestKey:  make(map[RequestKey]map[OwnerKey]JwksSource),
	}

	requestA := Request{URL: "https://issuer.example/shared"}
	requestB := Request{URL: "https://issuer.example/new"}
	keyA := requestA.Key()
	keyB := requestB.Key()

	movingOwner := JwksSource{
		OwnerKey:   testOwner("one"),
		RequestKey: keyA,
		Request:    requestA,
		TTL:        5 * time.Minute,
	}
	stayingOwner := JwksSource{
		OwnerKey:   testOwner("two"),
		RequestKey: keyA,
		Request:    requestA,
		TTL:        10 * time.Minute,
	}

	store.applyOwnerUpdate(movingOwner)
	store.applyOwnerUpdate(stayingOwner)
	update := store.applyOwnerUpdate(JwksSource{
		OwnerKey:   movingOwner.OwnerKey,
		RequestKey: keyB,
		Request:    requestB,
		TTL:        movingOwner.TTL,
	})

	if assert.Len(t, update.actions, 2) {
		assert.False(t, update.actions[0].delete)
		if assert.NotNil(t, update.actions[0].upsert) {
			assert.Equal(t, keyA, update.actions[0].upsert.RequestKey)
			assert.Equal(t, stayingOwner.TTL, update.actions[0].upsert.TTL)
		}

		assert.False(t, update.actions[1].delete)
		if assert.NotNil(t, update.actions[1].upsert) {
			assert.Equal(t, keyB, update.actions[1].upsert.RequestKey)
			assert.Equal(t, movingOwner.TTL, update.actions[1].upsert.TTL)
		}
	}
}

func testOwner(name string) OwnerKey {
	return JwksOwnerID{
		Kind:      OwnerKindPolicy,
		Namespace: "default",
		Name:      name,
		Path:      "spec.targetRefs[0].traffic.jwtAuthentication.providers[0].jwks.remote",
	}
}
