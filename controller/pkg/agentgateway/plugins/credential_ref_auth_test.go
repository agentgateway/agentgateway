package plugins

import (
	"errors"
	"fmt"
	"strings"
	"testing"

	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/test"
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
	stop := test.NewStop(t)
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAwsAuthResolvesConfiguredCredentialRef"), krt.WithStop(stop))
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

func TestAwsAuthPropagatesAssumeRoleSessionNameAndTags(t *testing.T) {
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAwsAuthPropagatesAssumeRoleSessionNameAndTags"))
	ctx := simpleAuthPolicyCtx(
		&AgwCollections{
			Secrets: secrets,
		}, kubeutils.NewSecretCredentialResolver(secrets))

	policy, err := buildAwsAuthPolicy(ctx, &agentgateway.AwsAuth{
		AssumeRole: &agentgateway.AwsAssumeRole{
			RoleArn:     "arn:aws:iam::111122223333:role/bedrock-team-acme-payments",
			SessionName: new("acme-payments-invoice-processor"),
			Tags: []agentgateway.AwsSessionTag{
				{Key: "Team", Value: new("acme-payments")},
				{Key: "App", Value: new("invoice-processor")},
			},
		},
	}, "default")
	assert.NoError(t, err)

	assumeRole := policy.GetAws().GetAssumeRole()
	assert.Equal(t, assumeRole != nil, true)
	assert.Equal(t, assumeRole.GetRoleArn(), "arn:aws:iam::111122223333:role/bedrock-team-acme-payments")
	assert.Equal(t, assumeRole.GetSessionName(), "acme-payments-invoice-processor")

	tags := assumeRole.GetTags()
	assert.Equal(t, len(tags), 2)
	assert.Equal(t, tags[0].GetKey(), "Team")
	assert.Equal(t, tags[0].GetValue(), "acme-payments")
	assert.Equal(t, tags[1].GetKey(), "App")
	assert.Equal(t, tags[1].GetValue(), "invoice-processor")
}

func TestAwsAuthPropagatesDynamicSessionTags(t *testing.T) {
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAwsAuthPropagatesDynamicSessionTags"))
	ctx := simpleAuthPolicyCtx(
		&AgwCollections{
			Secrets: secrets,
		}, kubeutils.NewSecretCredentialResolver(secrets))

	expression := agentgateway.CELExpression(`request.headers["x-app"]`)
	policy, err := buildAwsAuthPolicy(ctx, &agentgateway.AwsAuth{
		AssumeRole: &agentgateway.AwsAssumeRole{
			RoleArn: "arn:aws:iam::111122223333:role/bedrock-caller",
			Tags: []agentgateway.AwsSessionTag{
				{Key: "Team", Value: new("acme-payments")},
				{Key: "App", Expression: &expression},
			},
		},
	}, "default")
	assert.NoError(t, err)

	tags := policy.GetAws().GetAssumeRole().GetTags()
	assert.Equal(t, len(tags), 2)
	assert.Equal(t, tags[0].GetKey(), "Team")
	assert.Equal(t, tags[0].GetValue(), "acme-payments")
	assert.Equal(t, tags[0].GetExpression(), "")
	assert.Equal(t, tags[1].GetKey(), "App")
	assert.Equal(t, tags[1].GetValue(), "")
	assert.Equal(t, tags[1].GetExpression(), `request.headers["x-app"]`)
}

func TestAwsAuthPropagatesDynamicSessionName(t *testing.T) {
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAwsAuthPropagatesDynamicSessionName"))
	ctx := simpleAuthPolicyCtx(
		&AgwCollections{
			Secrets: secrets,
		}, kubeutils.NewSecretCredentialResolver(secrets))

	expression := agentgateway.CELExpression(`jwt.sub`)
	policy, err := buildAwsAuthPolicy(ctx, &agentgateway.AwsAuth{
		AssumeRole: &agentgateway.AwsAssumeRole{
			RoleArn:               "arn:aws:iam::111122223333:role/bedrock-caller",
			SessionNameExpression: &expression,
		},
	}, "default")
	assert.NoError(t, err)

	assumeRole := policy.GetAws().GetAssumeRole()
	assert.Equal(t, assumeRole != nil, true)
	assert.Equal(t, assumeRole.GetSessionName(), "")
	assert.Equal(t, assumeRole.GetSessionNameExpression(), "jwt.sub")
}

func TestAwsAuthAssumeRoleOmitsUnsetSessionNameAndTags(t *testing.T) {
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAwsAuthAssumeRoleOmitsUnsetSessionNameAndTags"))
	ctx := simpleAuthPolicyCtx(
		&AgwCollections{
			Secrets: secrets,
		}, kubeutils.NewSecretCredentialResolver(secrets))

	policy, err := buildAwsAuthPolicy(ctx, &agentgateway.AwsAuth{
		AssumeRole: &agentgateway.AwsAssumeRole{
			RoleArn: "arn:aws:iam::111122223333:role/backend",
		},
	}, "default")
	assert.NoError(t, err)

	assumeRole := policy.GetAws().GetAssumeRole()
	assert.Equal(t, assumeRole != nil, true)
	assert.Equal(t, assumeRole.GetSessionName(), "")
	assert.Equal(t, len(assumeRole.GetTags()), 0)
}

func TestAzureAuthResolvesConfiguredCredentialRef(t *testing.T) {
	stop := test.NewStop(t)
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAzureAuthResolvesConfiguredCredentialRef"), krt.WithStop(stop))
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

func TestAzureAuthBuildsExplicitAndImplicitConfigs(t *testing.T) {
	stop := test.NewStop(t)
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, nil, krt.WithName("plugins/TestAzureAuthBuildsExplicitAndImplicitConfigs"), krt.WithStop(stop))
	ctx := simpleAuthPolicyCtx(&AgwCollections{
		Secrets: secrets,
	}, kubeutils.NewSecretCredentialResolver(secrets))

	t.Run("workloadIdentity", func(t *testing.T) {
		policy, err := buildAzureAuthPolicy(ctx, &agentgateway.AzureAuth{
			WorkloadIdentity: &agentgateway.AzureWorkloadIdentity{},
		}, "default")
		assert.NoError(t, err)
		explicit := policy.GetAzure().GetExplicitConfig()
		assert.Equal(t, explicit != nil, true)
		assert.Equal(t, explicit.GetWorkloadIdentityCredential() != nil, true)
	})

	t.Run("implicit when no credential source is set", func(t *testing.T) {
		policy, err := buildAzureAuthPolicy(ctx, &agentgateway.AzureAuth{}, "default")
		assert.NoError(t, err)
		assert.Equal(t, policy.GetAzure().GetImplicit() != nil, true)
	})
}

func TestBasicAuthCanUseInjectedCredentialResolver(t *testing.T) {
	stop := test.NewStop(t)
	configMap := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "basic-auth",
		},
		Data: map[string]string{
			"users": "alice:hash",
		},
	}
	configMaps := krt.NewStaticCollection[*corev1.ConfigMap](nil, []*corev1.ConfigMap{configMap}, krt.WithName("plugins/TestBasicAuthCanUseInjectedCredentialResolver"), krt.WithStop(stop))
	ctx := simpleAuthPolicyCtx(nil, configMapCredentialResolver{configMaps: configMaps})

	policy, err := processBasicAuthenticationPolicy(ctx, &agentgateway.BasicAuthentication{
		SecretRef: &agentgateway.LocalSecretKeyRef{
			Name:  "basic-auth",
			Group: "example.agentgateway.dev",
			Kind:  "ConfigMapCredential",
			Key:   new("users"),
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
	stop := test.NewStop(t)
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "basic-auth",
		},
		Data: map[string][]byte{
			".htaccess": []byte("bob:hash"),
		},
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("plugins/TestBasicAuthFallsBackToSecretResolverWithInjectedCredentialResolver"), krt.WithStop(stop))
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
		SecretRef: &agentgateway.LocalSecretKeyRef{
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
	stop := test.NewStop(t)
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "basic-auth",
		},
		Data: map[string][]byte{
			".htaccess": []byte("bob:hash"),
		},
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("plugins/TestBasicAuthCustomResolverDoesNotImplicitlyFallbackToSecret"), krt.WithStop(stop))
	ctx := simpleAuthPolicyCtx(&AgwCollections{
		Secrets: secrets,
	}, configMapCredentialResolver{})

	_, err := processBasicAuthenticationPolicy(ctx, &agentgateway.BasicAuthentication{
		SecretRef: &agentgateway.LocalSecretKeyRef{
			Name: "basic-auth",
			Kind: "Secret",
		},
	}, nil, "base", types.NamespacedName{Namespace: "default", Name: "policy"})
	if !errors.Is(err, kubeutils.ErrUnsupportedCredentialKind) {
		t.Fatalf("processBasicAuthenticationPolicy() error = %v, want ErrUnsupportedCredentialKind", err)
	}
}

func TestBackendAuthCustomKeyRejectsEmptyValue(t *testing.T) {
	stop := test.NewStop(t)
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "backend-auth",
		},
		Data: map[string][]byte{
			"token": []byte("  "),
		},
	}
	secrets := krt.NewStaticCollection[*corev1.Secret](nil, []*corev1.Secret{secret}, krt.WithName("plugins/TestBackendAuthCustomKeyRejectsEmptyValue"), krt.WithStop(stop))
	ctx := simpleAuthPolicyCtx(&AgwCollections{
		Secrets: secrets,
	}, kubeutils.NewSecretCredentialResolver(secrets))
	policy := &agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Namespace: "default",
			Name:      "backend-auth",
		},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Backend: &agentgateway.BackendFull{
				BackendSimple: agentgateway.BackendSimple{
					Auth: &agentgateway.BackendAuth{
						SecretRef: &agentgateway.LocalSecretKeyRef{
							Name: "backend-auth",
							Key:  new("token"),
						},
					},
				},
			},
		},
	}

	_, err := translateBackendAuth(ctx, policy, "default/backend-auth")
	if err == nil || !strings.Contains(err.Error(), "missing token value") {
		t.Fatalf("translateBackendAuth() error = %v, want missing token error", err)
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
