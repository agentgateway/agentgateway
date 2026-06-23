package deployer

import (
	"context"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	appsv1 "k8s.io/api/apps/v1"
	autoscalingv2 "k8s.io/api/autoscaling/v2"
	corev1 "k8s.io/api/core/v1"
	policyv1 "k8s.io/api/policy/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"k8s.io/apimachinery/pkg/api/resource"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/annotations"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	agwplugins "github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/schemes"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
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

// TestGatewayIRFromInternalPorts guards the IR-fallback path (used when a Gateway
// isn't yet in the IR): internal (routing-only) ports must be populated from the
// annotation so GetPortsValues excludes them from the Service/container ports.
func TestGatewayIRFromInternalPorts(t *testing.T) {
	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:        "gw",
			Namespace:   "ns",
			Annotations: map[string]string{annotations.InternalPorts: "8080"},
		},
		Spec: gwv1.GatewaySpec{
			Listeners: []gwv1.Listener{
				{Name: "internal", Port: 8080, Protocol: gwv1.HTTPProtocolType},
				{Name: "public", Port: 8443, Protocol: gwv1.HTTPProtocolType},
			},
		},
	}

	ir := GatewayIRFrom(gw, "example.com/controller")
	assert.True(t, ir.InternalPorts.Contains(8080), "internal port 8080 should be recorded")
	assert.False(t, ir.InternalPorts.Contains(8443), "public port 8443 should not be internal")

	ports := GetPortsValues(ir, 0)
	got := map[int32]bool{}
	for _, p := range ports {
		got[*p.Port] = true
	}
	assert.False(t, got[8080], "internal port 8080 must not be exposed via Service/container ports")
	assert.True(t, got[8443], "public port 8443 should be exposed")
}

func TestAgentgatewayParametersApplier_ApplyToHelmValues_Image(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersConfigs: agentgateway.AgentgatewayParametersConfigs{
				Image: &agentgateway.Image{
					Registry:   new("custom.registry.io"),
					Repository: new("custom/agentgateway"),
					Tag:        new("v1.0.0"),
				},
			},
		},
	}

	applier := NewAgentgatewayParametersApplier(params)
	vals := &HelmConfig{
		Agentgateway: &AgentgatewayHelmGateway{},
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
	vals := &HelmConfig{
		Agentgateway: &AgentgatewayHelmGateway{},
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
	vals := &HelmConfig{
		Agentgateway: &AgentgatewayHelmGateway{},
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
	vals := &HelmConfig{
		Agentgateway: &AgentgatewayHelmGateway{},
	}

	applier.ApplyToHelmValues(vals)

	require.Len(t, vals.Agentgateway.Env, 2)
	assert.Equal(t, "SESSION_KEY", vals.Agentgateway.Env[0].Name)
	assert.Equal(t, "inline-key", vals.Agentgateway.Env[0].Value)
	assert.Equal(t, "CUSTOM_VAR", vals.Agentgateway.Env[1].Name)
}

func TestApplyManagedSessionKeyDefaults_UsesUserProvidedSessionKey(t *testing.T) {
	vals := &AgentgatewayHelmGateway{
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

func TestResolvedParameters_ResolveWorkloadKind(t *testing.T) {
	tests := []struct {
		desc     string
		resolved *resolvedParameters
		want     agentgateway.AgentgatewayParametersWorkloadKind
	}{
		{
			desc:     "default Deployment",
			resolved: &resolvedParameters{},
			want:     agentgateway.AgentgatewayParametersWorkloadDeployment,
		},
		{
			desc: "GatewayClass DaemonSet",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{},
				),
			},
			want: agentgateway.AgentgatewayParametersWorkloadDaemonSet,
		},
		{
			desc: "Gateway DaemonSet",
			resolved: &resolvedParameters{
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{},
				),
			},
			want: agentgateway.AgentgatewayParametersWorkloadDaemonSet,
		},
		{
			desc: "Gateway overrides Deployment to DaemonSet",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{},
				),
			},
			want: agentgateway.AgentgatewayParametersWorkloadDaemonSet,
		},
		{
			desc: "Gateway overrides DaemonSet to Deployment",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{},
				),
			},
			want: agentgateway.AgentgatewayParametersWorkloadDeployment,
		},
	}

	for _, tt := range tests {
		t.Run(tt.desc, func(t *testing.T) {
			got, err := tt.resolved.resolveWorkloadKind()
			require.NoError(t, err)
			assert.Equal(t, tt.want, got)
		})
	}
}

func TestResolvedParameters_ValidateWorkloadOverlays(t *testing.T) {
	overlay := &agentgateway.KubernetesResourceOverlay{}
	tests := []struct {
		desc            string
		resolved        *resolvedParameters
		wantErrContains []string
	}{
		{
			desc: "resolved Deployment accepts Deployment and HPA overlays",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					"",
					agentgateway.AgentgatewayParametersOverlays{
						Deployment:          overlay,
						Service:             overlay,
						PodDisruptionBudget: overlay,
					},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{
						HorizontalPodAutoscaler: overlay,
						ServiceAccount:          overlay,
					},
				),
			},
		},
		{
			desc: "resolved DaemonSet accepts DaemonSet overlays",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{
						DaemonSet: overlay,
						Service:   overlay,
					},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					"",
					agentgateway.AgentgatewayParametersOverlays{
						DaemonSet:           overlay,
						PodDisruptionBudget: overlay,
					},
				),
			},
		},
		{
			desc: "resolved Deployment rejects DaemonSet overlay",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{DaemonSet: overlay},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{},
				),
			},
			wantErrContains: []string{"GatewayClass", "class-agwp", "daemonSet", "Deployment"},
		},
		{
			desc: "resolved Deployment rejects Gateway DaemonSet overlay",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{DaemonSet: overlay},
				),
			},
			wantErrContains: []string{"Gateway", "gateway-agwp", "daemonSet", "Deployment"},
		},
		{
			desc: "resolved DaemonSet rejects Deployment overlay",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{Deployment: overlay},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{},
				),
			},
			wantErrContains: []string{"GatewayClass", "class-agwp", "deployment", "DaemonSet"},
		},
		{
			desc: "resolved DaemonSet rejects Gateway Deployment overlay",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{Deployment: overlay},
				),
			},
			wantErrContains: []string{"Gateway", "gateway-agwp", "deployment", "DaemonSet"},
		},
		{
			desc: "resolved DaemonSet rejects GatewayClass HPA overlay",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					agentgateway.AgentgatewayParametersWorkloadDeployment,
					agentgateway.AgentgatewayParametersOverlays{
						HorizontalPodAutoscaler: overlay,
					},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{},
				),
			},
			wantErrContains: []string{
				"GatewayClass",
				"class-agwp",
				"horizontalPodAutoscaler",
				"DaemonSet",
			},
		},
		{
			desc: "resolved DaemonSet rejects HPA overlay",
			resolved: &resolvedParameters{
				gatewayClassAGWP: agentgatewayParametersWithWorkloadKind(
					"class-agwp",
					"",
					agentgateway.AgentgatewayParametersOverlays{},
				),
				gatewayAGWP: agentgatewayParametersWithWorkloadKind(
					"gateway-agwp",
					agentgateway.AgentgatewayParametersWorkloadDaemonSet,
					agentgateway.AgentgatewayParametersOverlays{
						HorizontalPodAutoscaler: overlay,
					},
				),
			},
			wantErrContains: []string{
				"Gateway",
				"gateway-agwp",
				"horizontalPodAutoscaler",
				"DaemonSet",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.desc, func(t *testing.T) {
			err := tt.resolved.validateWorkloadOverlays()
			if len(tt.wantErrContains) == 0 {
				require.NoError(t, err)
				return
			}

			require.Error(t, err)
			for _, want := range tt.wantErrContains {
				assert.Contains(t, err.Error(), want)
			}
		})
	}
}

func agentgatewayParametersWithWorkloadKind(
	name string,
	kind agentgateway.AgentgatewayParametersWorkloadKind,
	overlays agentgateway.AgentgatewayParametersOverlays,
) *agentgateway.AgentgatewayParameters {
	params := &agentgateway.AgentgatewayParameters{
		ObjectMeta: metav1.ObjectMeta{
			Name:      name,
			Namespace: "default",
		},
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: overlays,
		},
	}
	if kind != "" {
		params.Spec.Workload = &agentgateway.AgentgatewayParametersWorkload{Kind: kind}
	}

	return params
}

func TestAgentgatewayParametersApplier_ApplyOverlaysToObjects(t *testing.T) {
	specPatch := []byte(`{
		"replicas": 3
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
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

// TestGetObjsToDeploy_MergesSupportResourceOverlays verifies GatewayClass and Gateway overlays are
// merged into one support resource per kind, with Gateway overlays taking precedence.
func TestGetObjsToDeploy_MergesSupportResourceOverlays(t *testing.T) {
	const (
		gatewayName     = "gw"
		namespace       = "default"
		classParamsName = "class-params"
		gwParamsName    = "gateway-params"
	)
	classMinAvailable := []byte(`{"minAvailable": 1}`)
	gatewayMinAvailable := []byte(`{"minAvailable": 2}`)
	classHPA := []byte(`{"minReplicas": 2, "maxReplicas": 5}`)
	gatewayHPA := []byte(`{"maxReplicas": 7}`)
	paramsNamespace := gwv1.Namespace(namespace)
	listenerProtocol := gwv1.HTTPProtocolType

	gw := &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      gatewayName,
			Namespace: namespace,
		},
		Spec: gwv1.GatewaySpec{
			GatewayClassName: gwv1.ObjectName(wellknown.DefaultAgwClassName),
			Infrastructure: &gwv1.GatewayInfrastructure{
				ParametersRef: &gwv1.LocalParametersReference{
					Group: agentgateway.GroupName,
					Kind:  gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind),
					Name:  gwParamsName,
				},
			},
			Listeners: []gwv1.Listener{{
				Name:     "http",
				Protocol: listenerProtocol,
				Port:     8080,
			}},
		},
	}
	gw.SetGroupVersionKind(wellknown.GatewayGVK)
	gwc := &gwv1.GatewayClass{
		ObjectMeta: metav1.ObjectMeta{Name: wellknown.DefaultAgwClassName},
		Spec: gwv1.GatewayClassSpec{
			ControllerName: gwv1.GatewayController(wellknown.DefaultAgwControllerName),
			ParametersRef: &gwv1.ParametersReference{
				Group:     agentgateway.GroupName,
				Kind:      gwv1.Kind(wellknown.AgentgatewayParametersGVK.Kind),
				Name:      classParamsName,
				Namespace: &paramsNamespace,
			},
		},
	}
	classParams := &agentgateway.AgentgatewayParameters{
		ObjectMeta: metav1.ObjectMeta{Name: classParamsName, Namespace: namespace},
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				PodDisruptionBudget: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{"shared": "class"},
					},
					Spec: &apiextensionsv1.JSON{Raw: classMinAvailable},
				},
				HorizontalPodAutoscaler: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{"shared": "class"},
					},
					Spec: &apiextensionsv1.JSON{Raw: classHPA},
				},
			},
		},
	}
	gwParams := &agentgateway.AgentgatewayParameters{
		ObjectMeta: metav1.ObjectMeta{Name: gwParamsName, Namespace: namespace},
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				PodDisruptionBudget: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{"shared": "gateway"},
					},
					Spec: &apiextensionsv1.JSON{Raw: gatewayMinAvailable},
				},
				HorizontalPodAutoscaler: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{"shared": "gateway"},
					},
					Spec: &apiextensionsv1.JSON{Raw: gatewayHPA},
				},
			},
		},
	}

	fakeClient := fake.NewClient(t, gw, gwc, classParams, gwParams)
	inputs := &Inputs{
		ImageDefaults: &agentgateway.Image{
			Registry:   new("cr.agentgateway.dev"),
			Repository: new("agentgateway"),
			Tag:        new("latest"),
		},
		ControlPlane: ControlPlaneInfo{
			XdsHost:          "agentgateway",
			AgwXdsPort:       15000,
			XdsTLSSecretName: "xds-tls",
			ControlPlaneNs:   "agentgateway-system",
		},
		NoListenersDummyPort:       15021,
		AgentgatewayClassName:      wellknown.DefaultAgwClassName,
		AgentgatewayControllerName: wellknown.DefaultAgwControllerName,
		AgwCollections: &agwplugins.AgwCollections{
			ControllerName:      wellknown.DefaultAgwControllerName,
			GatewaysForDeployer: krt.NewStaticCollection[collections.GatewayForDeployer](nil, nil),
		},
	}
	gp := NewGatewayParameters(fakeClient, inputs).WithSessionKeyGenerator(func() (string, error) {
		return "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff", nil
	})
	stop := test.NewStop(t)
	fakeClient.RunAndWait(stop)
	deployer, err := NewGatewayDeployer(
		wellknown.DefaultAgwControllerName,
		wellknown.DefaultAgwClassName,
		schemes.DefaultScheme(),
		fakeClient,
		gp,
	)
	require.NoError(t, err)

	objs, err := deployer.GetObjsToDeploy(context.Background(), gw)
	require.NoError(t, err)

	pdbs := podDisruptionBudgetsFromDeployObjects(objs)
	hpas := horizontalPodAutoscalersFromDeployObjects(objs)
	require.Len(t, pdbs, 1)
	require.Len(t, hpas, 1)

	pdb := pdbs[0]
	assert.Equal(t, "gateway", pdb.Labels["shared"])
	require.NotNil(t, pdb.Spec.MinAvailable)
	assert.Equal(t, int32(2), pdb.Spec.MinAvailable.IntVal)

	hpa := hpas[0]
	assert.Equal(t, "gateway", hpa.Labels["shared"])
	require.NotNil(t, hpa.Spec.MinReplicas)
	assert.Equal(t, int32(2), *hpa.Spec.MinReplicas)
	assert.Equal(t, int32(7), hpa.Spec.MaxReplicas)
	assert.Equal(t, "Deployment", hpa.Spec.ScaleTargetRef.Kind)
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
	vals := &HelmConfig{
		Agentgateway: &AgentgatewayHelmGateway{},
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
	vals := &HelmConfig{
		Agentgateway: &AgentgatewayHelmGateway{},
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
	vals := &HelmConfig{
		Agentgateway: &AgentgatewayHelmGateway{},
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
	daemonSet := &appsv1.DaemonSet{}
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw-session-key",
			Namespace: "default",
		},
		Data: map[string][]byte{
			"key": []byte("00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff"),
		},
	}

	err := addSessionKeyChecksumAnnotation([]client.Object{deployment, daemonSet}, secret)
	require.NoError(t, err)
	checksum := "2a8abfa8cb9906290437854193ca6bca41d4d4e26d1d454bd66a35158095e737"
	require.NotNil(t, deployment.Spec.Template.Annotations)
	assert.Equal(t, checksum, deployment.Spec.Template.Annotations[sessionKeyChecksumAnnotation])
	require.NotNil(t, daemonSet.Spec.Template.Annotations)
	assert.Equal(t, checksum, daemonSet.Spec.Template.Annotations[sessionKeyChecksumAnnotation])
}

func podDisruptionBudgetsFromDeployObjects(objs []client.Object) []*policyv1.PodDisruptionBudget {
	pdbs := make([]*policyv1.PodDisruptionBudget, 0)
	for _, obj := range objs {
		pdb, ok := obj.(*policyv1.PodDisruptionBudget)
		if ok {
			pdbs = append(pdbs, pdb)
		}
	}
	return pdbs
}

func horizontalPodAutoscalersFromDeployObjects(objs []client.Object) []*autoscalingv2.HorizontalPodAutoscaler {
	hpas := make([]*autoscalingv2.HorizontalPodAutoscaler, 0)
	for _, obj := range objs {
		hpa, ok := obj.(*autoscalingv2.HorizontalPodAutoscaler)
		if ok {
			hpas = append(hpas, hpa)
		}
	}
	return hpas
}
