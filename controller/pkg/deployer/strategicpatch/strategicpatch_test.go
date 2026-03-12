package strategicpatch

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
	appsv1 "k8s.io/api/apps/v1"
	autoscalingv2 "k8s.io/api/autoscaling/v2"
	corev1 "k8s.io/api/core/v1"
	policyv1 "k8s.io/api/policy/v1"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/shared"
)

func TestOverlayApplier_ApplyOverlays_NilParams(t *testing.T) {
	applier := NewOverlayApplier(nil)
	objs := []client.Object{
		&appsv1.Deployment{
			TypeMeta: metav1.TypeMeta{
				APIVersion: "apps/v1",
				Kind:       "Deployment",
			},
			ObjectMeta: metav1.ObjectMeta{
				Name: "test-deployment",
			},
		},
	}

	result, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)
	assert.Len(t, result, 1)
}

func TestOverlayApplier_ApplyOverlays_MetadataLabels(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: &shared.KubernetesResourceOverlay{
					Metadata: &shared.ObjectMetadata{
						Labels: map[string]string{
							"custom-label": "custom-value",
						},
					},
				},
			},
		},
	}

	applier := NewOverlayApplier(params)
	deployment := &appsv1.Deployment{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "apps/v1",
			Kind:       "Deployment",
		},
		ObjectMeta: metav1.ObjectMeta{
			Name: "test-deployment",
			Labels: map[string]string{
				"existing-label": "existing-value",
			},
		},
	}
	objs := []client.Object{deployment}

	objs, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)

	result := objs[0].(*appsv1.Deployment)
	assert.Equal(t, "custom-value", result.Labels["custom-label"])
	assert.Equal(t, "existing-value", result.Labels["existing-label"])
}

func TestOverlayApplier_ApplyOverlays_MetadataAnnotations(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Service: &shared.KubernetesResourceOverlay{
					Metadata: &shared.ObjectMetadata{
						Annotations: map[string]string{
							"custom-annotation": "custom-value",
						},
					},
				},
			},
		},
	}

	applier := NewOverlayApplier(params)
	svc := &corev1.Service{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "v1",
			Kind:       "Service",
		},
		ObjectMeta: metav1.ObjectMeta{
			Name: "test-service",
		},
	}
	objs := []client.Object{svc}

	objs, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)

	result := objs[0].(*corev1.Service)
	assert.Equal(t, "custom-value", result.Annotations["custom-annotation"])
}

func TestOverlayApplier_ApplyOverlays_DeploymentSpec(t *testing.T) {
	// Test strategic merge patch for deployment spec
	specPatch := []byte(`{
		"replicas": 3,
		"template": {
			"spec": {
				"containers": [{
					"name": "agent-gateway",
					"resources": {
						"limits": {
							"memory": "512Mi"
						}
					}
				}]
			}
		}
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: &shared.KubernetesResourceOverlay{
					Spec: &apiextensionsv1.JSON{Raw: specPatch},
				},
			},
		},
	}

	applier := NewOverlayApplier(params)
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
			Template: corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{
						{
							Name:  "agent-gateway",
							Image: "cr.agentgateway.dev/agentgateway:latest",
						},
					},
				},
			},
		},
	}
	objs := []client.Object{deployment}

	objs, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)

	result := objs[0].(*appsv1.Deployment)
	assert.Equal(t, int32(3), *result.Spec.Replicas)
	assert.Equal(t, "cr.agentgateway.dev/agentgateway:latest", result.Spec.Template.Spec.Containers[0].Image)
	assert.NotNil(t, result.Spec.Template.Spec.Containers[0].Resources.Limits)
	assert.Equal(t, "512Mi", result.Spec.Template.Spec.Containers[0].Resources.Limits.Memory().String())
}

func TestOverlayApplier_ApplyOverlays_DeleteContainerWithPatchDirective(t *testing.T) {
	// Test strategic merge patch with $patch: delete directive
	specPatch := []byte(`{
		"template": {
			"spec": {
				"containers": [{
					"name": "sidecar",
					"$patch": "delete"
				}]
			}
		}
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: &shared.KubernetesResourceOverlay{
					Spec: &apiextensionsv1.JSON{Raw: specPatch},
				},
			},
		},
	}

	applier := NewOverlayApplier(params)
	deployment := &appsv1.Deployment{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "apps/v1",
			Kind:       "Deployment",
		},
		ObjectMeta: metav1.ObjectMeta{
			Name: "test-deployment",
		},
		Spec: appsv1.DeploymentSpec{
			Template: corev1.PodTemplateSpec{
				Spec: corev1.PodSpec{
					Containers: []corev1.Container{
						{
							Name:  "agent-gateway",
							Image: "cr.agentgateway.dev/agentgateway:latest",
						},
						{
							Name:  "sidecar",
							Image: "sidecar:latest",
						},
					},
				},
			},
		},
	}
	objs := []client.Object{deployment}

	objs, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)

	result := objs[0].(*appsv1.Deployment)
	require.Len(t, result.Spec.Template.Spec.Containers, 1)
	assert.Equal(t, "agent-gateway", result.Spec.Template.Spec.Containers[0].Name)
}

func TestOverlayApplier_ApplyOverlays_ServiceSpec(t *testing.T) {
	specPatch := []byte(`{
		"type": "NodePort"
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Service: &shared.KubernetesResourceOverlay{
					Spec: &apiextensionsv1.JSON{Raw: specPatch},
				},
			},
		},
	}

	applier := NewOverlayApplier(params)
	svc := &corev1.Service{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "v1",
			Kind:       "Service",
		},
		ObjectMeta: metav1.ObjectMeta{
			Name: "test-service",
		},
		Spec: corev1.ServiceSpec{
			Type: corev1.ServiceTypeLoadBalancer,
		},
	}
	objs := []client.Object{svc}

	objs, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)

	result := objs[0].(*corev1.Service)
	assert.Equal(t, corev1.ServiceTypeNodePort, result.Spec.Type)
}

func TestOverlayApplier_ApplyOverlays_MultipleObjects(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: &shared.KubernetesResourceOverlay{
					Metadata: &shared.ObjectMetadata{
						Labels: map[string]string{"app": "modified"},
					},
				},
				Service: &shared.KubernetesResourceOverlay{
					Metadata: &shared.ObjectMetadata{
						Labels: map[string]string{"svc": "modified"},
					},
				},
				ServiceAccount: &shared.KubernetesResourceOverlay{
					Metadata: &shared.ObjectMetadata{
						Labels: map[string]string{"sa": "modified"},
					},
				},
			},
		},
	}

	applier := NewOverlayApplier(params)
	objs := []client.Object{
		&appsv1.Deployment{
			TypeMeta:   metav1.TypeMeta{APIVersion: "apps/v1", Kind: "Deployment"},
			ObjectMeta: metav1.ObjectMeta{Name: "test-deployment"},
		},
		&corev1.Service{
			TypeMeta:   metav1.TypeMeta{APIVersion: "v1", Kind: "Service"},
			ObjectMeta: metav1.ObjectMeta{Name: "test-service"},
		},
		&corev1.ServiceAccount{
			TypeMeta:   metav1.TypeMeta{APIVersion: "v1", Kind: "ServiceAccount"},
			ObjectMeta: metav1.ObjectMeta{Name: "test-sa"},
		},
		&corev1.ConfigMap{
			TypeMeta:   metav1.TypeMeta{APIVersion: "v1", Kind: "ConfigMap"},
			ObjectMeta: metav1.ObjectMeta{Name: "test-cm"},
		},
	}

	objs, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)

	// Check deployment
	deploy := objs[0].(*appsv1.Deployment)
	assert.Equal(t, "modified", deploy.Labels["app"])

	// Check service
	svc := objs[1].(*corev1.Service)
	assert.Equal(t, "modified", svc.Labels["svc"])

	// Check service account
	sa := objs[2].(*corev1.ServiceAccount)
	assert.Equal(t, "modified", sa.Labels["sa"])

	// Check configmap (should be unchanged, no overlay for it)
	cm := objs[3].(*corev1.ConfigMap)
	assert.Empty(t, cm.Labels)
}

// deploymentWithLabels returns a Deployment carrying the given labels and a
// matching label selector, suitable for use as the base object when testing
// PDB / HPA / VPA creation.
func deploymentWithLabels(labels map[string]string) *appsv1.Deployment {
	return &appsv1.Deployment{
		TypeMeta: metav1.TypeMeta{APIVersion: "apps/v1", Kind: "Deployment"},
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gw",
			Namespace: "default",
			Labels:    labels,
		},
		Spec: appsv1.DeploymentSpec{
			Selector: &metav1.LabelSelector{MatchLabels: labels},
		},
	}
}

var gatewayLabels = map[string]string{
	"app.kubernetes.io/instance":                   "gw",
	"app.kubernetes.io/managed-by":                 "kgateway",
	"app.kubernetes.io/name":                       "gw",
	"app.kubernetes.io/version":                    "1.0.0-dev",
	"gateway.networking.k8s.io/gateway-class-name": "agentgateway",
	"gateway.networking.k8s.io/gateway-name":       "gw",
	"kgateway":                                     "kube-gateway",
}

func TestCreatePodDisruptionBudget_InheritsDeploymentLabels(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &shared.KubernetesResourceOverlay{}

	obj, err := createPodDisruptionBudget(dep, overlay)
	require.NoError(t, err)

	pdb := obj.(*policyv1.PodDisruptionBudget)
	assert.Equal(t, gatewayLabels, pdb.GetLabels())
}

func TestCreatePodDisruptionBudget_OverlayLabelsMergeOnTop(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &shared.KubernetesResourceOverlay{
		Metadata: &shared.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}

	obj, err := createPodDisruptionBudget(dep, overlay)
	require.NoError(t, err)

	pdb := obj.(*policyv1.PodDisruptionBudget)
	assert.Equal(t, "label", pdb.GetLabels()["extra"])
	for k, v := range gatewayLabels {
		assert.Equal(t, v, pdb.GetLabels()[k])
	}
}

func TestCreateHorizontalPodAutoscaler_InheritsDeploymentLabels(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &shared.KubernetesResourceOverlay{}

	obj, err := createHorizontalPodAutoscaler(dep, overlay)
	require.NoError(t, err)

	hpa := obj.(*autoscalingv2.HorizontalPodAutoscaler)
	assert.Equal(t, gatewayLabels, hpa.GetLabels())
}

func TestCreateHorizontalPodAutoscaler_OverlayLabelsMergeOnTop(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &shared.KubernetesResourceOverlay{
		Metadata: &shared.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}

	obj, err := createHorizontalPodAutoscaler(dep, overlay)
	require.NoError(t, err)

	hpa := obj.(*autoscalingv2.HorizontalPodAutoscaler)
	assert.Equal(t, "label", hpa.GetLabels()["extra"])
	for k, v := range gatewayLabels {
		assert.Equal(t, v, hpa.GetLabels()[k])
	}
}

func TestCreateVerticalPodAutoscaler_InheritsDeploymentLabels(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &shared.KubernetesResourceOverlay{}

	obj, err := createVerticalPodAutoscaler(dep, overlay)
	require.NoError(t, err)

	vpa := obj.(*unstructured.Unstructured)
	assert.Equal(t, gatewayLabels, vpa.GetLabels())
}

func TestCreateVerticalPodAutoscaler_OverlayLabelsMergeOnTop(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &shared.KubernetesResourceOverlay{
		Metadata: &shared.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}

	obj, err := createVerticalPodAutoscaler(dep, overlay)
	require.NoError(t, err)

	vpa := obj.(*unstructured.Unstructured)
	assert.Equal(t, "label", vpa.GetLabels()["extra"])
	for k, v := range gatewayLabels {
		assert.Equal(t, v, vpa.GetLabels()[k])
	}
}

func TestCreatePodDisruptionBudget_DeploymentLabelsNotMutated(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &shared.KubernetesResourceOverlay{
		Metadata: &shared.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}

	_, err := createPodDisruptionBudget(dep, overlay)
	require.NoError(t, err)

	// The original deployment labels must not have been mutated.
	assert.NotContains(t, dep.GetLabels(), "extra")
}
