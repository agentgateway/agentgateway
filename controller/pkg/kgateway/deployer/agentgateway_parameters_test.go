package deployer

import (
	"context"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/test"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"k8s.io/apimachinery/pkg/api/resource"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
)

func newSyncedSecretClient(t *testing.T, objects ...client.Object) kclient.Client[*corev1.Secret] {
	t.Helper()

	fakeClient := fake.NewClient(t, objects...)
	secretClient := kclient.NewFiltered[*corev1.Secret](fakeClient, kclient.Filter{
		ObjectFilter: fakeClient.ObjectFilter(),
	})
	stop := test.NewStop(t)
	fakeClient.RunAndWait(stop)
	kube.WaitForCacheSync("test", stop, secretClient.HasSynced)
	return secretClient
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_Image(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Image: &agentgateway.Image{
					Registry:   ptr.To("custom.registry.io"),
					Repository: ptr.To("custom/agentgateway"),
					Tag:        ptr.To("v1.0.0"),
				},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)

	require.NotNil(t, vals.Agentgateway.Image)
	assert.Equal(t, "custom.registry.io", *vals.Agentgateway.Image.Registry)
	assert.Equal(t, "custom/agentgateway", *vals.Agentgateway.Image.Repository)
	assert.Equal(t, "v1.0.0", *vals.Agentgateway.Image.Tag)
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_Resources(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Resources: &corev1.ResourceRequirements{
					Limits: corev1.ResourceList{
						corev1.ResourceMemory: resource.MustParse("512Mi"),
						corev1.ResourceCPU:    resource.MustParse("500m"),
					},
					Requests: corev1.ResourceList{
						corev1.ResourceMemory: resource.MustParse("256Mi"),
						corev1.ResourceCPU:    resource.MustParse("250m"),
					},
				},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)

	require.NotNil(t, vals.Agentgateway.Resources)
	assert.Equal(t, "512Mi", vals.Agentgateway.Resources.Limits.Memory().String())
	assert.Equal(t, "500m", vals.Agentgateway.Resources.Limits.Cpu().String())
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_Env(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Env: []corev1.EnvVar{
					{Name: "CUSTOM_VAR", Value: "custom_value"},
					{Name: "ANOTHER_VAR", Value: "another_value"},
				},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)

	require.Len(t, vals.Agentgateway.Env, 2)
	assert.Equal(t, "CUSTOM_VAR", vals.Agentgateway.Env[0].Name)
	assert.Equal(t, "ANOTHER_VAR", vals.Agentgateway.Env[1].Name)
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_PreservesSessionKeyEnvVar(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Env: []corev1.EnvVar{
					{Name: "SESSION_KEY", Value: "inline-key"},
					{Name: "CUSTOM_VAR", Value: "custom_value"},
				},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)

	require.Len(t, vals.Agentgateway.Env, 2)
	assert.Equal(t, "SESSION_KEY", vals.Agentgateway.Env[0].Name)
	assert.Equal(t, "inline-key", vals.Agentgateway.Env[0].Value)
	assert.Equal(t, "CUSTOM_VAR", vals.Agentgateway.Env[1].Name)
}

func TestApplyManagedSessionKeyDefaults_UsesUserProvidedSessionKey(t *testing.T) {
	vals := &deployer.AgentgatewayHelmGateway{
		AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
			Env: []corev1.EnvVar{
				{Name: "SESSION_KEY", Value: "inline-key"},
			},
		},
	}

	applyManagedSessionKeyDefaults(vals, "gw")

	assert.Nil(t, vals.SessionKeySecretName)
}

func TestUsesManagedSessionKeyResolvedParameters_GatewayEnvDisablesManagedSecret(t *testing.T) {
	resolved := &resolvedParameters{
		gatewayClassAGWP: &agentgateway.AgentgatewayParameters{
			Spec: agentgateway.AgentgatewayParametersSpec{
				AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
					Env: []corev1.EnvVar{{Name: "RUST_LOG", Value: "info"}},
				},
			},
		},
		gatewayAGWP: &agentgateway.AgentgatewayParameters{
			Spec: agentgateway.AgentgatewayParametersSpec{
				AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
					Env: []corev1.EnvVar{{Name: "SESSION_KEY", Value: "inline-key"}},
				},
			},
		},
	}

	assert.False(t, usesManagedSessionKeyResolvedParameters(resolved))
}

func TestAgentgatewayParametersApplier_ApplyOverlaysToObjects(t *testing.T) {
	specPatch := []byte(`{
		"replicas": 3
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: &shared.KubernetesResourceOverlay{
					Metadata: &shared.ObjectMetadata{
						Labels: map[string]string{
							"overlay-label": "overlay-value",
						},
					},
					Spec: &apiextensionsv1.JSON{Raw: specPatch},
				},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)

	deployment := &appsv1.Deployment{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "apps/v1",
			Kind:       "Deployment",
		},
		ObjectMeta: metav1.ObjectMeta{
			Name: "test-deployment",
		},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr.To[int32](1),
		},
	}
	objs := []client.Object{deployment}

	objs, err := applier.ApplyOverlaysToObjects(objs)
	require.NoError(t, err)

	result := objs[0].(*appsv1.Deployment)
	assert.Equal(t, int32(3), *result.Spec.Replicas)
	assert.Equal(t, "overlay-value", result.Labels["overlay-label"])
}

func TestAgentgatewayParametersApplier_ApplyOverlaysToObjects_NilParams(t *testing.T) {
	applier := NewAgentgatewayParametersApplier(nil)

	deployment := &appsv1.Deployment{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "apps/v1",
			Kind:       "Deployment",
		},
		ObjectMeta: metav1.ObjectMeta{
			Name: "test-deployment",
		},
		Spec: appsv1.DeploymentSpec{
			Replicas: ptr.To[int32](1),
		},
	}
	objs := []client.Object{deployment}

	objs, err := applier.ApplyOverlaysToObjects(objs)
	require.NoError(t, err)

	result := objs[0].(*appsv1.Deployment)
	assert.Equal(t, int32(1), *result.Spec.Replicas)
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_RawConfig(t *testing.T) {
	rawConfigJSON := []byte(`{
		"tracing": {
			"otlpEndpoint": "http://jaeger:4317"
		},
		"metrics": {
			"enabled": true
		}
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				RawConfig: &apiextensionsv1.JSON{Raw: rawConfigJSON},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)
	assert.Equal(t, vals.Agentgateway.RawConfig.Raw, rawConfigJSON)
}

// TestAgentgatewayParametersApplier_ApplyToHelmValues_NoAliasing verifies that
// applying GatewayClass AGWP followed by Gateway AGWP does not mutate the
// cached GatewayClass object. This reproduces a bug where the first Apply
// returned a pointer alias to configs.Resources, and the second Apply mutated
// that alias via maps.Copy when merging requests/limits.
func TestAgentgatewayParametersApplier_ApplyToHelmValues_NoAliasing(t *testing.T) {
	// Simulate the cached GatewayClass AGWP with resource limits.
	gatewayClassAGWP := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Resources: &corev1.ResourceRequirements{
					Limits: corev1.ResourceList{
						corev1.ResourceCPU:    resource.MustParse("500m"),
						corev1.ResourceMemory: resource.MustParse("512Mi"),
					},
				},
			},
		},
	}

	// Simulate the cached Gateway AGWP with resource requests.
	gatewayAGWP := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Resources: &corev1.ResourceRequirements{
					Requests: corev1.ResourceList{
						corev1.ResourceCPU:    resource.MustParse("250m"),
						corev1.ResourceMemory: resource.MustParse("128Mi"),
					},
				},
			},
		},
	}

	// Snapshot the original GatewayClass limits before merging.
	origGWCLimits := gatewayClassAGWP.Spec.Resources.Limits.DeepCopy()

	// Apply GatewayClass first, then Gateway — same order as GetValues.
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}
	NewAgentgatewayParametersApplier(gatewayClassAGWP).ApplyToHelmValues(vals)
	NewAgentgatewayParametersApplier(gatewayAGWP).ApplyToHelmValues(vals)

	// The merged result should have both the GWC limits and the GW requests.
	require.NotNil(t, vals.Agentgateway.Resources)
	assert.Equal(t, resource.MustParse("500m"), vals.Agentgateway.Resources.Limits[corev1.ResourceCPU],
		"merged result should contain GWC CPU limit")
	assert.Equal(t, resource.MustParse("250m"), vals.Agentgateway.Resources.Requests[corev1.ResourceCPU],
		"merged result should contain GW CPU request")
	assert.Equal(t, resource.MustParse("128Mi"), vals.Agentgateway.Resources.Requests[corev1.ResourceMemory],
		"merged result should contain GW memory request")

	// The cached GatewayClass object must NOT have been mutated.
	assert.Equal(t, origGWCLimits, gatewayClassAGWP.Spec.Resources.Limits,
		"cached GatewayClass Limits must not be mutated by subsequent Gateway merge")
	assert.Nil(t, gatewayClassAGWP.Spec.Resources.Requests,
		"cached GatewayClass Requests must remain nil")
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_RawConfigWithLogging(t *testing.T) {
	// rawConfig has logging.format, but typed Logging.Format should take precedence
	// (merging happens in helm template, but here we test both are passed through)
	rawConfigJSON := []byte(`{
		"logging": {
			"format": "json"
		},
		"tracing": {
			"otlpEndpoint": "http://jaeger:4317"
		}
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Logging: &agentgateway.AgentgatewayParametersLogging{
					Format: agentgateway.AgentgatewayParametersLoggingText,
				},
				RawConfig: &apiextensionsv1.JSON{Raw: rawConfigJSON},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)

	// Both should be set - merging happens in helm template
	assert.Equal(t, "text", string(vals.Agentgateway.Logging.Format))
	assert.Equal(t, vals.Agentgateway.RawConfig.Raw, rawConfigJSON)
}

func TestBuildSessionKeySecret_UsesExistingValidKey(t *testing.T) {
	const existingKey = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw-session-key",
			Namespace: "default",
		},
		Data: map[string][]byte{
			"key": []byte(existingKey),
		},
	}
	generator := &agentgatewayParametersHelmValuesGenerator{
		secretClient: newSyncedSecretClient(t, secret),
		sessionKeyGen: func() (string, error) {
			return "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100", nil
		},
	}
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw",
			Namespace: "default",
		},
		Spec: gwv1.GatewaySpec{
			GatewayClassName: "agentgateway",
		},
	}

	managedSecret, err := generator.buildSessionKeySecret(context.Background(), gw, "gw-session-key")
	require.NoError(t, err)
	require.NotNil(t, managedSecret)
	assert.Equal(t, existingKey, string(managedSecret.Data["key"]))
	assert.Equal(t, corev1.SecretTypeOpaque, managedSecret.Type)
	assert.Equal(t, "gw-session-key", managedSecret.Name)
}

func TestBuildSessionKeySecret_RejectsInvalidExistingKey(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw-session-key",
			Namespace: "default",
		},
		Data: map[string][]byte{
			"key": []byte("not-a-valid-key"),
		},
	}
	generator := &agentgatewayParametersHelmValuesGenerator{
		secretClient: newSyncedSecretClient(t, secret),
		sessionKeyGen: func() (string, error) {
			return "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff", nil
		},
	}
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw",
			Namespace: "default",
		},
	}

	_, err := generator.buildSessionKeySecret(context.Background(), gw, "gw-session-key")
	require.Error(t, err)
	assert.Contains(t, err.Error(), "contains an invalid key")
}

func TestAddSessionKeyChecksumAnnotation(t *testing.T) {
	deployment := &appsv1.Deployment{}
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw-session-key",
			Namespace: "default",
		},
		Data: map[string][]byte{
			"key": []byte("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"),
		},
	}

	err := addSessionKeyChecksumAnnotation([]client.Object{deployment}, secret)
	require.NoError(t, err)
	require.NotNil(t, deployment.Spec.Template.Annotations)
	assert.Equal(t,
		"2a8abfa8cb9906290437854193ca6bca41d4d4e26d1d454bd66a35158095e737",
		deployment.Spec.Template.Annotations[sessionKeyChecksumAnnotation],
	)
}
