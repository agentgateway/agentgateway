package jwks

import (
	"testing"

	"github.com/stretchr/testify/assert"
	corev1 "k8s.io/api/core/v1"
)

func TestPlanConfigMapSyncKeepsCanonicalConfigMap(t *testing.T) {
	artifact := Artifact{
		RequestKey: Request{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	plan := planConfigMapSync(artifact.RequestKey, nil, DefaultJwksStorePrefix, func(requestKey RequestKey) (Artifact, bool) {
		if requestKey == artifact.RequestKey {
			return artifact, true
		}
		return Artifact{}, false
	})

	if assert.NotNil(t, plan.artifact) {
		assert.Equal(t, artifact, *plan.artifact)
	}
	assert.Equal(t, JwksConfigMapName(DefaultJwksStorePrefix, artifact.RequestKey), plan.upsertName)
	assert.Empty(t, plan.deleteNames)
}

func TestPlanConfigMapSyncDeletesInactiveConfigMap(t *testing.T) {
	artifact := Artifact{
		RequestKey: Request{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	cmName := JwksConfigMapName(DefaultJwksStorePrefix, artifact.RequestKey)
	existingCm := configMapWithArtifact(t, cmName, "agentgateway-system", artifact)

	plan := planConfigMapSync(artifact.RequestKey, []*corev1.ConfigMap{existingCm}, DefaultJwksStorePrefix, func(RequestKey) (Artifact, bool) {
		return Artifact{}, false
	})

	assert.Nil(t, plan.artifact)
	assert.Empty(t, plan.upsertName)
	assert.Equal(t, []string{cmName}, plan.deleteNames)
}

func TestPlanConfigMapSyncNoopsWhenConfigMapIsAlreadyGone(t *testing.T) {
	requestKey := Request{URL: "https://issuer.example/jwks"}.Key()

	plan := planConfigMapSync(requestKey, nil, DefaultJwksStorePrefix, func(RequestKey) (Artifact, bool) {
		return Artifact{}, false
	})

	assert.Nil(t, plan.artifact)
	assert.Empty(t, plan.upsertName)
	assert.Empty(t, plan.deleteNames)
}

func TestPlanConfigMapSyncDeletesNonCanonicalConfigMapsForActiveRequest(t *testing.T) {
	artifact := Artifact{
		RequestKey: Request{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}
	canonicalName := JwksConfigMapName(DefaultJwksStorePrefix, artifact.RequestKey)
	legacyName := "jwks-store-legacy-name"
	plan := planConfigMapSync(
		artifact.RequestKey,
		[]*corev1.ConfigMap{
			configMapWithArtifact(t, canonicalName, "agentgateway-system", artifact),
			configMapWithArtifact(t, legacyName, "agentgateway-system", artifact),
		},
		DefaultJwksStorePrefix,
		func(requestKey RequestKey) (Artifact, bool) {
			if requestKey == artifact.RequestKey {
				return artifact, true
			}
			return Artifact{}, false
		},
	)

	if assert.NotNil(t, plan.artifact) {
		assert.Equal(t, artifact, *plan.artifact)
	}
	assert.Equal(t, canonicalName, plan.upsertName)
	assert.Equal(t, []string{legacyName}, plan.deleteNames)
}

func configMapWithArtifact(t *testing.T, name, namespace string, artifact Artifact) *corev1.ConfigMap {
	t.Helper()

	cm := &corev1.ConfigMap{
		Data: map[string]string{},
	}
	cm.Name = name
	cm.Namespace = namespace
	if err := SetJwksInConfigMap(cm, artifact); err != nil {
		t.Fatalf("SetJwksInConfigMap() error = %v", err)
	}
	return cm
}
