package plugins

import (
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/api"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

func TestApiKeyAuthPassesHashedValuesThrough(t *testing.T) {
	const (
		plainValue  = "plain-key"
		sha256Value = "sha256:4c806362b613f7496abf284146efd31da90e4b16169fe001841ca17290f427c4"
		bcryptValue = "$2b$12$abcdefghijklmnopqrstuv0123456789012345678901234567890ab"
	)
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Namespace: "default", Name: "api-keys"},
		Data: map[string][]byte{
			"plain":  []byte(plainValue),
			"hashed": []byte(sha256Value),
			"bcrypt": []byte(bcryptValue),
		},
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("plugins/TestApiKeyAuthPassesHashedValuesThrough"))
	ctx := simpleAuthPolicyCtx(&AgwCollections{Secrets: secrets}, kubeutils.NewSecretCredentialResolver(secrets))

	policy, err := processAPIKeyAuthenticationPolicy(ctx, &agentgateway.APIKeyAuthentication{
		SecretRef: &agentgateway.LocalSecretObjectRef{Name: "api-keys", Kind: "Secret"},
	}, nil, "base", types.NamespacedName{Namespace: "default", Name: "policy"})
	assert.NoError(t, err)

	keys := slices.Map(policy.GetTraffic().GetApiKeyAuth().GetApiKeys(), func(u *api.TrafficPolicySpec_APIKey_User) string { return u.GetKey() })
	assert.Equal(t, slices.Contains(keys, plainValue), true)
	assert.Equal(t, slices.Contains(keys, sha256Value), true)
	assert.Equal(t, slices.Contains(keys, bcryptValue), true)
}
