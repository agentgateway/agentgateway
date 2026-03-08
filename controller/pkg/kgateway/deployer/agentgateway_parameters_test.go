package deployer

import (
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
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
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/deployer"
)

func newSessionKeyGeneratorForTest(
	t *testing.T,
	sessionKeyGen func() (string, error),
	objects ...client.Object,
) *agentgatewayParametersHelmValuesGenerator {
	t.Helper()

	_, generator := newSessionKeyGeneratorHarness(t, sessionKeyGen, objects...)
	return generator
}

func newSessionKeyGeneratorHarness(
	t *testing.T,
	sessionKeyGen func() (string, error),
	objects ...client.Object,
) (apiclient.Client, *agentgatewayParametersHelmValuesGenerator) {
	t.Helper()

	fakeClient := fake.NewClient(t, objects...)
	stop := test.NewStop(t)
	fakeClient.RunAndWait(stop)
	generator := newAgentgatewayParametersHelmValuesGenerator(fakeClient, &deployer.Inputs{})
	generator.sessionKeyGen = sessionKeyGen
	return fakeClient, generator
}

func mustManagedSessionKeyPayload(t *testing.T, keyring *managedSessionKeyring) []byte {
	t.Helper()

	payload, err := keyring.Serialize()
	require.NoError(t, err)
	return payload
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

func TestAgentgatewayParametersApplier_ApplyToHelmValues_FiltersReservedSessionKeyEnvVars(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Env: []corev1.EnvVar{
					{Name: "SESSION_KEY", Value: "inline-key"},
					{Name: sessionKeyringFileEnvVar, Value: "/tmp/override"},
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

	require.Len(t, vals.Agentgateway.Env, 1)
	assert.Equal(t, "CUSTOM_VAR", vals.Agentgateway.Env[0].Name)
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_StripsReservedSessionRawConfig(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				RawConfig: &apiextensionsv1.JSON{Raw: []byte(`{
					"config": {
						"session": {
							"key": "should-be-ignored"
						},
						"tracing": {
							"otlpEndpoint": "http://jaeger:4317"
						}
					}
				}`)},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &deployer.HelmConfig{
		Agentgateway: &deployer.AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)
	require.NotNil(t, vals.Agentgateway.RawConfig)

	var doc map[string]any
	require.NoError(t, json.Unmarshal(vals.Agentgateway.RawConfig.Raw, &doc))
	config, ok := doc["config"].(map[string]any)
	require.True(t, ok)
	assert.NotContains(t, config, "session")
	assert.Contains(t, config, "tracing")
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

func TestBuildSessionKeySecret(t *testing.T) {
	const existingKey = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
	const nextKey = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100"
	newGateway := func() *gwv1.Gateway {
		return &gwv1.Gateway{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "gw",
				Namespace: "default",
				UID:       "gateway-uid",
			},
			Spec: gwv1.GatewaySpec{
				GatewayClassName: "agentgateway",
			},
		}
	}
	managedSecretForGateway := func(t *testing.T, gw *gwv1.Gateway, primary string) *corev1.Secret {
		t.Helper()
		return &corev1.Secret{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "gw-session-key",
				Namespace: gw.Namespace,
				Labels: map[string]string{
					managedSessionKeyLabel: managedSessionKeyLabelValue,
				},
				Annotations: map[string]string{
					managedSessionKeyGatewayNameAnnotation:      gw.Name,
					managedSessionKeyGatewayNamespaceAnnotation: gw.Namespace,
					managedSessionKeyGatewayUIDAnnotation:       string(gw.UID),
				},
			},
			Data: map[string][]byte{
				managedSessionKeyDataKey: mustManagedSessionKeyPayload(t, &managedSessionKeyring{
					Version: managedSessionKeyVersion,
					Primary: primary,
				}),
			},
		}
	}

	tests := []struct {
		name      string
		setup     func(t *testing.T) (*gwv1.Gateway, []client.Object)
		run       func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error)
		keys      []string
		callCount int
		assert    func(t *testing.T, secret *corev1.Secret, err error, callCount int)
	}{
		{
			name: "reuses existing valid managed keyring",
			setup: func(t *testing.T) (*gwv1.Gateway, []client.Object) {
				gw := newGateway()
				return gw, []client.Object{managedSecretForGateway(t, gw, existingKey)}
			},
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				return generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
			},
			keys: []string{nextKey},
			assert: func(t *testing.T, secret *corev1.Secret, err error, callCount int) {
				require.NoError(t, err)
				require.NotNil(t, secret)
				keyring, parseErr := parseManagedSessionKeyring(secret.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, existingKey, keyring.Primary)
				assert.Equal(t, corev1.SecretTypeOpaque, secret.Type)
				assert.Equal(t, "gw-session-key", secret.Name)
				assert.Zero(t, callCount)
			},
		},
		{
			name: "rejects foreign secret collision",
			setup: func(t *testing.T) (*gwv1.Gateway, []client.Object) {
				gw := newGateway()
				return gw, []client.Object{
					&corev1.Secret{
						ObjectMeta: metav1.ObjectMeta{
							Name:      "gw-session-key",
							Namespace: gw.Namespace,
						},
						Data: map[string][]byte{
							managedSessionKeyDataKey: []byte(`{"version":"v1","primary":"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"}`),
						},
					},
				}
			},
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				return generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
			},
			keys: []string{nextKey},
			assert: func(t *testing.T, secret *corev1.Secret, err error, callCount int) {
				require.Error(t, err)
				var conflictErr *SessionKeyConflictError
				require.ErrorAs(t, err, &conflictErr)
				assert.Nil(t, secret)
				assert.Zero(t, callCount)
			},
		},
		{
			name: "repairs invalid managed secret",
			setup: func(t *testing.T) (*gwv1.Gateway, []client.Object) {
				gw := newGateway()
				return gw, []client.Object{
					&corev1.Secret{
						ObjectMeta: metav1.ObjectMeta{
							Name:      "gw-session-key",
							Namespace: gw.Namespace,
							Labels: map[string]string{
								managedSessionKeyLabel: managedSessionKeyLabelValue,
							},
							Annotations: map[string]string{
								managedSessionKeyGatewayNameAnnotation:      gw.Name,
								managedSessionKeyGatewayNamespaceAnnotation: gw.Namespace,
								managedSessionKeyGatewayUIDAnnotation:       string(gw.UID),
							},
						},
						Data: map[string][]byte{
							managedSessionKeyDataKey: []byte("not-json"),
						},
					},
				}
			},
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				return generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
			},
			keys:      []string{existingKey},
			callCount: 1,
			assert: func(t *testing.T, secret *corev1.Secret, err error, callCount int) {
				require.NoError(t, err)
				keyring, parseErr := parseManagedSessionKeyring(secret.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, existingKey, keyring.Primary)
				assert.Equal(t, 1, callCount)
			},
		},
		{
			name: "repeated reconcile is stable",
			setup: func(t *testing.T) (*gwv1.Gateway, []client.Object) {
				return newGateway(), nil
			},
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				if _, err := generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key"); err != nil {
					return nil, err
				}
				return generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
			},
			keys:      []string{existingKey, nextKey},
			callCount: 1,
			assert: func(t *testing.T, secret *corev1.Secret, err error, callCount int) {
				require.NoError(t, err)
				keyring, parseErr := parseManagedSessionKeyring(secret.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, existingKey, keyring.Primary)
				assert.Equal(t, 1, callCount)
			},
		},
		{
			name: "rotates managed keyring",
			setup: func(t *testing.T) (*gwv1.Gateway, []client.Object) {
				gw := newGateway()
				gw.Annotations = map[string]string{
					managedSessionKeyRotationAnnotation: "rotate-1",
				}
				return gw, []client.Object{managedSecretForGateway(t, gw, existingKey)}
			},
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				return generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
			},
			keys:      []string{nextKey},
			callCount: 1,
			assert: func(t *testing.T, secret *corev1.Secret, err error, callCount int) {
				require.NoError(t, err)
				keyring, parseErr := parseManagedSessionKeyring(secret.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, nextKey, keyring.Primary)
				assert.Equal(t, []string{existingKey}, keyring.Previous)
				assert.Equal(t, "rotate-1", secret.Annotations[managedSessionKeyHandledRotationAnnotation])
				assert.Equal(t, 1, callCount)
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gw, objects := tt.setup(t)
			callCount := 0
			keys := tt.keys
			if len(keys) == 0 {
				keys = []string{existingKey}
			}
			generator := newSessionKeyGeneratorForTest(t, func() (string, error) {
				key := keys[min(callCount, len(keys)-1)]
				callCount++
				return key, nil
			}, objects...)
			secret, err := tt.run(t, generator, gw)
			tt.assert(t, secret, err, callCount)
		})
	}
}

func TestBuildSessionKeySecret_LiveAPIState(t *testing.T) {
	const existingKey = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"
	const nextKey = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100"

	newGateway := func() *gwv1.Gateway {
		return &gwv1.Gateway{
			ObjectMeta: metav1.ObjectMeta{
				Name:      "gw",
				Namespace: "default",
				UID:       "gateway-uid",
			},
			Spec: gwv1.GatewaySpec{
				GatewayClassName: "agentgateway",
			},
		}
	}

	tests := []struct {
		name    string
		prepare func(t *testing.T, cli apiclient.Client, gw *gwv1.Gateway)
		run     func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error)
		assert  func(t *testing.T, cli apiclient.Client, secret *corev1.Secret, err error)
	}{
		{
			name: "create persists and second reconcile reuses api state",
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				first, err := generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
				require.NoError(t, err)
				generator.sessionKeyGen = func() (string, error) {
					return nextKey, nil
				}
				second, err := generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
				require.NoError(t, err)

				firstKeyring, parseErr := parseManagedSessionKeyring(first.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				secondKeyring, parseErr := parseManagedSessionKeyring(second.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, firstKeyring.Primary, secondKeyring.Primary)
				return second, nil
			},
			assert: func(t *testing.T, cli apiclient.Client, secret *corev1.Secret, err error) {
				require.NoError(t, err)
				assert.Equal(t, corev1.SchemeGroupVersion.String(), secret.APIVersion)
				assert.Equal(t, "Secret", secret.Kind)
				assert.Empty(t, secret.ResourceVersion)
				assert.Empty(t, secret.UID)
				assert.True(t, secret.CreationTimestamp.IsZero())
				assert.Nil(t, secret.ManagedFields)
				liveSecret, getErr := cli.Kube().CoreV1().Secrets("default").Get(t.Context(), "gw-session-key", metav1.GetOptions{})
				require.NoError(t, getErr)
				keyring, parseErr := parseManagedSessionKeyring(liveSecret.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, existingKey, keyring.Primary)
				assert.Equal(t, secret.Data[managedSessionKeyDataKey], liveSecret.Data[managedSessionKeyDataKey])
			},
		},
		{
			name: "live api corruption is repaired",
			prepare: func(t *testing.T, cli apiclient.Client, gw *gwv1.Gateway) {
				_, err := cli.Kube().CoreV1().Secrets(gw.Namespace).Create(t.Context(), &corev1.Secret{
					ObjectMeta: metav1.ObjectMeta{
						Name:      "gw-session-key",
						Namespace: gw.Namespace,
						Labels: map[string]string{
							managedSessionKeyLabel: managedSessionKeyLabelValue,
						},
						Annotations: map[string]string{
							managedSessionKeyGatewayNameAnnotation:      gw.Name,
							managedSessionKeyGatewayNamespaceAnnotation: gw.Namespace,
							managedSessionKeyGatewayUIDAnnotation:       string(gw.UID),
						},
					},
					Type: corev1.SecretTypeOpaque,
					Data: map[string][]byte{
						managedSessionKeyDataKey: []byte("corrupted"),
					},
				}, metav1.CreateOptions{})
				require.NoError(t, err)
			},
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				return generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
			},
			assert: func(t *testing.T, cli apiclient.Client, secret *corev1.Secret, err error) {
				require.NoError(t, err)
				assert.Equal(t, corev1.SchemeGroupVersion.String(), secret.APIVersion)
				assert.Equal(t, "Secret", secret.Kind)
				assert.Empty(t, secret.ResourceVersion)
				assert.Empty(t, secret.UID)
				assert.True(t, secret.CreationTimestamp.IsZero())
				assert.Nil(t, secret.ManagedFields)
				keyring, parseErr := parseManagedSessionKeyring(secret.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, existingKey, keyring.Primary)

				liveSecret, getErr := cli.Kube().CoreV1().Secrets("default").Get(t.Context(), "gw-session-key", metav1.GetOptions{})
				require.NoError(t, getErr)
				liveKeyring, parseErr := parseManagedSessionKeyring(liveSecret.Data[managedSessionKeyDataKey])
				require.NoError(t, parseErr)
				assert.Equal(t, existingKey, liveKeyring.Primary)
			},
		},
		{
			name: "live api foreign collision fails",
			prepare: func(t *testing.T, cli apiclient.Client, gw *gwv1.Gateway) {
				_, err := cli.Kube().CoreV1().Secrets(gw.Namespace).Create(t.Context(), &corev1.Secret{
					ObjectMeta: metav1.ObjectMeta{
						Name:      "gw-session-key",
						Namespace: gw.Namespace,
					},
					Type: corev1.SecretTypeOpaque,
					Data: map[string][]byte{
						managedSessionKeyDataKey: []byte(`{"version":"v1","primary":"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"}`),
					},
				}, metav1.CreateOptions{})
				require.NoError(t, err)
			},
			run: func(t *testing.T, generator *agentgatewayParametersHelmValuesGenerator, gw *gwv1.Gateway) (*corev1.Secret, error) {
				return generator.buildSessionKeySecret(t.Context(), gw, "gw-session-key")
			},
			assert: func(t *testing.T, cli apiclient.Client, secret *corev1.Secret, err error) {
				require.Error(t, err)
				var conflictErr *SessionKeyConflictError
				require.ErrorAs(t, err, &conflictErr)
				assert.Nil(t, secret)
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cli, generator := newSessionKeyGeneratorHarness(t, func() (string, error) {
				return existingKey, nil
			})
			gw := newGateway()
			if tt.prepare != nil {
				tt.prepare(t, cli, gw)
			}
			secret, err := tt.run(t, generator, gw)
			tt.assert(t, cli, secret, err)
		})
	}
}

func TestAddSessionKeyChecksumAnnotation(t *testing.T) {
	deployment := &appsv1.Deployment{}
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw-session-key",
			Namespace: "default",
		},
		Data: map[string][]byte{
			managedSessionKeyDataKey: mustManagedSessionKeyPayload(t, &managedSessionKeyring{
				Version: managedSessionKeyVersion,
				Primary: "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
			}),
		},
	}

	err := addSessionKeyChecksumAnnotation([]client.Object{deployment}, secret)
	require.NoError(t, err)
	require.NotNil(t, deployment.Spec.Template.Annotations)
	assert.Equal(t,
		"f8dc99f51dfac136be0f82b86b24b0bf9fb595c462bed19362eef8125a3ac0f8",
		deployment.Spec.Template.Annotations[sessionKeyChecksumAnnotation],
	)
}

func TestEnforceManagedSessionKeyDeploymentWiring_OverwritesOverlayMutations(t *testing.T) {
	deployment := &appsv1.Deployment{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw",
			Namespace: "default",
		},
		Spec: appsv1.DeploymentSpec{
			Template: corev1.PodTemplateSpec{
				ObjectMeta: metav1.ObjectMeta{
					Annotations: map[string]string{
						sessionKeyChecksumAnnotation: "stale",
					},
				},
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{{
						Name: "agentgateway",
						Env: []corev1.EnvVar{
							{
								Name:  "SESSION_KEY",
								Value: "inline-key",
							},
							{
								Name:  sessionKeyringFileEnvVar,
								Value: "/tmp/override",
							},
						},
						VolumeMounts: []corev1.VolumeMount{{
							Name:      managedSessionKeyVolumeName,
							MountPath: "/tmp/override",
						}},
					}},
					Volumes: []corev1.Volume{{
						Name: "session-key",
						VolumeSource: corev1.VolumeSource{
							Secret: &corev1.SecretVolumeSource{
								SecretName: "stale",
							},
						},
					}},
				},
			},
		},
	}
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw-session-key",
			Namespace: "default",
		},
		Data: map[string][]byte{
			managedSessionKeyDataKey: mustManagedSessionKeyPayload(t, &managedSessionKeyring{
				Version: managedSessionKeyVersion,
				Primary: "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
			}),
		},
	}

	err := enforceManagedSessionKeyDeploymentWiring([]client.Object{deployment}, secret)
	require.NoError(t, err)

	env := deployment.Spec.Template.Spec.Containers[0].Env
	require.Len(t, env, 1)
	assert.Equal(t, sessionKeyringFileEnvVar, env[0].Name)
	assert.Equal(t, managedSessionKeyMountPath+"/"+managedSessionKeyFileName, env[0].Value)
	assert.Nil(t, env[0].ValueFrom)
	mounts := deployment.Spec.Template.Spec.Containers[0].VolumeMounts
	require.Len(t, mounts, 1)
	assert.Equal(t, managedSessionKeyVolumeName, mounts[0].Name)
	assert.Equal(t, managedSessionKeyMountPath, mounts[0].MountPath)
	assert.True(t, mounts[0].ReadOnly)
	volumes := deployment.Spec.Template.Spec.Volumes
	require.Len(t, volumes, 1)
	assert.Equal(t, managedSessionKeyVolumeName, volumes[0].Name)
	require.NotNil(t, volumes[0].Secret)
	assert.Equal(t, secret.Name, volumes[0].Secret.SecretName)
	require.Len(t, volumes[0].Secret.Items, 1)
	assert.Equal(t, managedSessionKeyDataKey, volumes[0].Secret.Items[0].Key)
	assert.Equal(t, managedSessionKeyFileName, volumes[0].Secret.Items[0].Path)
	assert.Equal(t,
		"f8dc99f51dfac136be0f82b86b24b0bf9fb595c462bed19362eef8125a3ac0f8",
		deployment.Spec.Template.Annotations[sessionKeyChecksumAnnotation],
	)
}
