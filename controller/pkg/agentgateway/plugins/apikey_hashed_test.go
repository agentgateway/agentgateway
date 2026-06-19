package plugins

import (
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

func apiKeyAuthFromSecret(t *testing.T, name string, data map[string][]byte) (*api.Policy, error) {
	t.Helper()
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: name},
		Data:       data,
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("plugins/"+name))
	ctx := simpleAuthPolicyCtx(&AgwCollections{Secrets: secrets}, kubeutils.NewSecretCredentialResolver(secrets))
	return processAPIKeyAuthenticationPolicy(ctx, &agentgateway.APIKeyAuthentication{
		SecretRef: &agentgateway.LocalSecretObjectRef{Name: gwv1.ObjectName(name), Kind: "Secret"},
	}, nil, "base", types.NamespacedName{Namespace: "default", Name: "policy"})
}

func TestApiKeyAuthPassesHashedValuesThrough(t *testing.T) {
	const (
		sha256Value = "sha256:4c806362b613f7496abf284146efd31da90e4b16169fe001841ca17290f427c4"
		bcryptValue = "$2b$04$ZWehanXB64VdH1a950nsTuj9edryGc3I3OicMkcHvQWqN7zQOqUni"
	)
	policy, err := apiKeyAuthFromSecret(t, "api-keys", map[string][]byte{
		"plain":  []byte("plain-key"),
		"hashed": []byte(`{"keyHash": "` + sha256Value + `"}`),
		"bcrypt": []byte(`{"keyHash": "` + bcryptValue + `"}`),
	})
	assert.NoError(t, err)

	users := policy.GetTraffic().GetApiKeyAuth().GetApiKeys()
	plainKeys := slices.Map(users, func(u *api.TrafficPolicySpec_APIKey_User) string { return u.GetKey() })
	hashes := slices.Map(users, func(u *api.TrafficPolicySpec_APIKey_User) string { return u.GetKeyHash() })

	assert.Equal(t, slices.Contains(plainKeys, "plain-key"), true)
	assert.Equal(t, slices.Contains(hashes, sha256Value), true)
	assert.Equal(t, slices.Contains(hashes, bcryptValue), true)
}

func TestApiKeyAuthRejectsInvalidKeyHash(t *testing.T) {
	_, err := apiKeyAuthFromSecret(t, "api-keys-bad", map[string][]byte{
		"bad": []byte(`{"keyHash": "sha256:not-a-valid-digest"}`),
	})
	assert.Error(t, err)
}

func TestApiKeyAuthRejectsBothKeyAndKeyHash(t *testing.T) {
	_, err := apiKeyAuthFromSecret(t, "api-keys-both", map[string][]byte{
		"both": []byte(`{"key": "k", "keyHash": "sha256:4c806362b613f7496abf284146efd31da90e4b16169fe001841ca17290f427c4"}`),
	})
	assert.Error(t, err)
}
