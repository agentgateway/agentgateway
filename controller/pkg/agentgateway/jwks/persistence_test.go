package jwks

import (
	"testing"

	"github.com/stretchr/testify/assert"
	corev1 "k8s.io/api/core/v1"
)

func TestJwksFromConfigMapRejectsLegacyPayload(t *testing.T) {
	cm := &corev1.ConfigMap{
		Data: map[string]string{
			configMapKey: `{"https://issuer.example/jwks":{"keys":[]}}`,
		},
	}

	_, err := JwksFromConfigMap(cm)

	assert.Error(t, err)
	assert.Contains(t, err.Error(), "unsupported jwks artifact version")
}

func TestSetJwksInConfigMapInitializesDataMap(t *testing.T) {
	cm := &corev1.ConfigMap{}
	artifact := Artifact{
		RequestKey: Request{URL: "https://issuer.example/jwks"}.Key(),
		URL:        "https://issuer.example/jwks",
		JwksJSON:   `{"keys":[]}`,
	}

	err := SetJwksInConfigMap(cm, artifact)

	assert.NoError(t, err)
	assert.NotNil(t, cm.Data)
	assert.Contains(t, cm.Data, configMapKey)
}
