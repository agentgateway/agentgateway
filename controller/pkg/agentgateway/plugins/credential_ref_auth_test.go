package plugins

import (
	"errors"
	"fmt"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/kubeutils"
)

func simpleAuthPolicyCtx(col *AgwCollections, res kubeutils.CredentialResolver) PolicyCtx {
	return PolicyCtx{
		Krt:                krt.TestingDummyContext{},
		Collections:        col,
		CredentialResolver: res,
	}
}

func TestAwsAuthResolvesConfiguredCredentialRef(t *testing.T) {
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAwsAuthResolvesConfiguredCredentialRef"))
	ctx := simpleAuthPolicyCtx(
		&AgwCollections{
			Secrets: secrets,
		}, kubeutils.NewSecretCredentialResolver(secrets))

	policy, err := buildAwsAuthPolicy(ctx, &agentgateway.AwsAuth{}, "default")
	assert.NoError(t, err)
	assert.Equal(t, policy != nil, true)

	_, err = buildAwsAuthPolicy(ctx, &agentgateway.AwsAuth{
		SecretRef: &agentgateway.LocalSecretObjectRef{
			Group: "agentgateway.dev",
			Kind:  "FileCredential",
			Name:  "file",
		},
	}, "default")
	if !errors.Is(err, kubeutils.ErrUnsupportedCredentialKind) {
		t.Fatalf("buildAwsAuthPolicy() error = %v, want ErrUnsupportedCredentialKind", err)
	}
}

func TestAzureAuthResolvesConfiguredCredentialRef(t *testing.T) {
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAzureAuthResolvesConfiguredCredentialRef"))
	ctx := simpleAuthPolicyCtx(&AgwCollections{
		Secrets: secrets,
	}, kubeutils.NewSecretCredentialResolver(secrets))

	_, err := buildAzureAuthPolicy(ctx, &agentgateway.AzureAuth{
		SecretRef: &agentgateway.LocalSecretObjectRef{
			Group: "agentgateway.dev",
			Kind:  "FileCredential",
			Name:  "file",
		},
	}, "default")
	if !errors.Is(err, kubeutils.ErrUnsupportedCredentialKind) {
		t.Fatalf("buildAzureAuthPolicy() error = %v, want ErrUnsupportedCredentialKind", err)
	}
}

func TestBasicAuthCanUseInjectedCredentialResolver(t *testing.T) {
	configMap := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "basic-auth",
		},
		Data: map[string]string{
			".htaccess": "alice:hash",
		},
	}
	configMaps := krt.NewStaticCollection[*corev1.ConfigMap](nil, []*corev1.ConfigMap{configMap}, krt.WithName("plugins/TestBasicAuthCanUseInjectedCredentialResolver"))
	ctx := simpleAuthPolicyCtx(nil, configMapCredentialResolver{configMaps: configMaps})

	policy, err := processBasicAuthenticationPolicy(ctx, &agentgateway.BasicAuthentication{
		SecretRef: &agentgateway.LocalSecretObjectRef{
			Name:  "basic-auth",
			Group: "example.agentgateway.dev",
			Kind:  "ConfigMapCredential",
		},
	}, nil, "base", types.NamespacedName{Namespace: "default", Name: "policy"})
	if err != nil {
		t.Fatalf("processBasicAuthenticationPolicy() error = %v, want nil", err)
	}
	if got := policy.GetTraffic().GetBasicAuth().HtpasswdContent; got != "alice:hash" {
		t.Fatalf("basic auth htpasswd content = %q, want %q", got, "alice:hash")
	}
}

func TestBasicAuthFallsBackToSecretResolverWithInjectedCredentialResolver(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "basic-auth",
		},
		Data: map[string][]byte{
			".htaccess": []byte("bob:hash"),
		},
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("plugins/TestBasicAuthFallsBackToSecretResolverWithInjectedCredentialResolver"))
	ctx := simpleAuthPolicyCtx(
		&AgwCollections{
			Secrets: secrets,
		},
		kubeutils.NewChainedCredentialResolver(
			configMapCredentialResolver{},
			kubeutils.NewSecretCredentialResolver(secrets),
		),
	)

	policy, err := processBasicAuthenticationPolicy(ctx, &agentgateway.BasicAuthentication{
		SecretRef: &agentgateway.LocalSecretObjectRef{
			Name: "basic-auth",
			Kind: "Secret",
		},
	}, nil, "base", types.NamespacedName{Namespace: "default", Name: "policy"})
	if err != nil {
		t.Fatalf("processBasicAuthenticationPolicy() error = %v, want nil", err)
	}
	if got := policy.GetTraffic().GetBasicAuth().HtpasswdContent; got != "bob:hash" {
		t.Fatalf("basic auth htpasswd content = %q, want %q", got, "bob:hash")
	}
}

func TestBasicAuthCustomResolverDoesNotImplicitlyFallbackToSecret(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "basic-auth",
		},
		Data: map[string][]byte{
			".htaccess": []byte("bob:hash"),
		},
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("plugins/TestBasicAuthCustomResolverDoesNotImplicitlyFallbackToSecret"))
	ctx := simpleAuthPolicyCtx(&AgwCollections{
		Secrets: secrets,
	}, configMapCredentialResolver{})

	_, err := processBasicAuthenticationPolicy(ctx, &agentgateway.BasicAuthentication{
		SecretRef: &agentgateway.LocalSecretObjectRef{
			Name: "basic-auth",
			Kind: "Secret",
		},
	}, nil, "base", types.NamespacedName{Namespace: "default", Name: "policy"})
	if !errors.Is(err, kubeutils.ErrUnsupportedCredentialKind) {
		t.Fatalf("processBasicAuthenticationPolicy() error = %v, want ErrUnsupportedCredentialKind", err)
	}
}

type configMapCredentialResolver struct {
	configMaps krt.Collection[*corev1.ConfigMap]
}

func (r configMapCredentialResolver) ResolveCredentialRef(krtctx krt.HandlerContext, ref agentgateway.LocalSecretObjectRef, namespace string) (map[string][]byte, error) {
	if ref.Group != "example.agentgateway.dev" || ref.Kind != "ConfigMapCredential" {
		return nil, fmt.Errorf("%w: %q/%q", kubeutils.ErrUnsupportedCredentialKind, ref.Group, ref.Kind)
	}
	configMap := ptr.Flatten(krt.FetchOne(krtctx, r.configMaps, krt.FilterKey(namespace+"/"+string(ref.Name))))
	if configMap == nil {
		return nil, fmt.Errorf("ConfigMap %s/%s not found", namespace, ref.Name)
	}
	data := make(map[string][]byte, len(configMap.Data))
	for k, v := range configMap.Data {
		data[k] = []byte(v)
	}
	return data, nil
}
