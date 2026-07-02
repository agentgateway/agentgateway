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
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
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
				Deployment: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
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
				Service: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
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
				Deployment: &agentgateway.KubernetesResourceOverlay{
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

func TestOverlayApplier_ApplyOverlays_DaemonSetSpec(t *testing.T) {
	specPatch := []byte(`{
		"updateStrategy": {
			"type": "RollingUpdate"
		},
		"template": {
			"spec": {
				"tolerations": [{
					"operator": "Exists"
				}]
			}
		}
	}`)

	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				DaemonSet: &agentgateway.KubernetesResourceOverlay{
					Spec: &apiextensionsv1.JSON{Raw: specPatch},
				},
			},
		},
	}

	applier := NewOverlayApplier(params)
	daemonSet := &appsv1.DaemonSet{
		TypeMeta: metav1.TypeMeta{
			APIVersion: "apps/v1",
			Kind:       "DaemonSet",
		},
		ObjectMeta: metav1.ObjectMeta{
			Name: "test-daemonset",
		},
		Spec: appsv1.DaemonSetSpec{
			UpdateStrategy: appsv1.DaemonSetUpdateStrategy{
				Type: appsv1.OnDeleteDaemonSetStrategyType,
			},
		},
	}
	objs := []client.Object{daemonSet}

	objs, err := applier.ApplyOverlays(objs)
	require.NoError(t, err)

	result := objs[0].(*appsv1.DaemonSet)
	assert.Equal(t, appsv1.RollingUpdateDaemonSetStrategyType, result.Spec.UpdateStrategy.Type)
	require.Len(t, result.Spec.Template.Spec.Tolerations, 1)
	assert.Equal(t, corev1.TolerationOpExists, result.Spec.Template.Spec.Tolerations[0].Operator)
}

func TestOverlayApplier_RejectsHPADaemonSetWorkload(t *testing.T) {
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				HorizontalPodAutoscaler: &agentgateway.KubernetesResourceOverlay{},
			},
		},
	}

	daemonSet := &appsv1.DaemonSet{
		TypeMeta: metav1.TypeMeta{APIVersion: "apps/v1", Kind: "DaemonSet"},
		ObjectMeta: metav1.ObjectMeta{
			Name:      "test-daemonset",
			Namespace: "default",
		},
		Spec: appsv1.DaemonSetSpec{},
	}

	applier := NewOverlayApplier(params)
	_, err := applier.ApplyOverlays([]client.Object{daemonSet})
	require.Error(t, err)
	assert.Contains(t, err.Error(), "horizontalPodAutoscaler overlay is not supported for DaemonSet workload")
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
				Deployment: &agentgateway.KubernetesResourceOverlay{
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
				Service: &agentgateway.KubernetesResourceOverlay{
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
				Deployment: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{"app": "modified"},
					},
				},
				Service: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{"svc": "modified"},
					},
				},
				ServiceAccount: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
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

func TestLayeredOverlayApplier_MergesPodDisruptionBudgetOverlays(t *testing.T) {
	classMinAvailable := []byte(`{"minAvailable": 1}`)
	gatewayMinAvailable := []byte(`{"minAvailable": 2}`)
	classParams := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				PodDisruptionBudget: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{
							"class-label": "class-value",
							"shared":      "class",
						},
					},
					Spec: &apiextensionsv1.JSON{Raw: classMinAvailable},
				},
			},
		},
	}
	gatewayParams := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				PodDisruptionBudget: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{
							"gateway-label": "gateway-value",
							"shared":        "gateway",
						},
					},
					Spec: &apiextensionsv1.JSON{Raw: gatewayMinAvailable},
				},
			},
		},
	}

	applier := NewLayeredOverlayApplier(classParams, gatewayParams)
	objs, err := applier.ApplyOverlays([]client.Object{deploymentWithLabels(gatewayLabels)})
	require.NoError(t, err)

	pdbs := podDisruptionBudgetsFromObjects(objs)
	require.Len(t, pdbs, 1)
	pdb := pdbs[0]
	assert.Equal(t, "class-value", pdb.Labels["class-label"])
	assert.Equal(t, "gateway-value", pdb.Labels["gateway-label"])
	assert.Equal(t, "gateway", pdb.Labels["shared"])
	require.NotNil(t, pdb.Spec.MinAvailable)
	assert.Equal(t, int32(2), pdb.Spec.MinAvailable.IntVal)
}

func TestLayeredOverlayApplier_MergesHorizontalPodAutoscalerOverlays(t *testing.T) {
	classSpec := []byte(`{"minReplicas": 2, "maxReplicas": 5}`)
	gatewaySpec := []byte(`{"maxReplicas": 7}`)
	classParams := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				HorizontalPodAutoscaler: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{
							"class-label": "class-value",
							"shared":      "class",
						},
					},
					Spec: &apiextensionsv1.JSON{Raw: classSpec},
				},
			},
		},
	}
	gatewayParams := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				HorizontalPodAutoscaler: &agentgateway.KubernetesResourceOverlay{
					Metadata: &agentgateway.ObjectMetadata{
						Labels: map[string]string{
							"gateway-label": "gateway-value",
							"shared":        "gateway",
						},
					},
					Spec: &apiextensionsv1.JSON{Raw: gatewaySpec},
				},
			},
		},
	}

	applier := NewLayeredOverlayApplier(classParams, gatewayParams)
	objs, err := applier.ApplyOverlays([]client.Object{deploymentWithLabels(gatewayLabels)})
	require.NoError(t, err)

	hpas := horizontalPodAutoscalersFromObjects(objs)
	require.Len(t, hpas, 1)
	hpa := hpas[0]
	assert.Equal(t, "class-value", hpa.Labels["class-label"])
	assert.Equal(t, "gateway-value", hpa.Labels["gateway-label"])
	assert.Equal(t, "gateway", hpa.Labels["shared"])
	require.NotNil(t, hpa.Spec.MinReplicas)
	assert.Equal(t, int32(2), *hpa.Spec.MinReplicas)
	assert.Equal(t, int32(7), hpa.Spec.MaxReplicas)
	assert.Equal(t, "apps/v1", hpa.Spec.ScaleTargetRef.APIVersion)
	assert.Equal(t, "Deployment", hpa.Spec.ScaleTargetRef.Kind)
	assert.Equal(t, "gw", hpa.Spec.ScaleTargetRef.Name)
}

func TestLayeredOverlayApplier_PodDisruptionBudgetUsesFinalWorkloadSelector(t *testing.T) {
	selectorPatch := []byte(`{
		"selector": {
			"matchLabels": {
				"app": "after"
			}
		}
	}`)
	params := &agentgateway.AgentgatewayParameters{
		Spec: agentgateway.AgentgatewayParametersSpec{
			AgentgatewayParametersOverlays: agentgateway.AgentgatewayParametersOverlays{
				Deployment: &agentgateway.KubernetesResourceOverlay{
					Spec: &apiextensionsv1.JSON{Raw: selectorPatch},
				},
				PodDisruptionBudget: &agentgateway.KubernetesResourceOverlay{},
			},
		},
	}
	deployment := deploymentWithLabels(map[string]string{"app": "before"})

	applier := NewLayeredOverlayApplier(params)
	objs, err := applier.ApplyOverlays([]client.Object{deployment})
	require.NoError(t, err)

	pdbs := podDisruptionBudgetsFromObjects(objs)
	require.Len(t, pdbs, 1)
	assert.Equal(t, map[string]string{"app": "after"}, pdbs[0].Spec.Selector.MatchLabels)
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
	"app.kubernetes.io/managed-by":                 "agentgateway",
	"app.kubernetes.io/name":                       "gw",
	"app.kubernetes.io/version":                    "1.0.0-dev",
	"gateway.networking.k8s.io/gateway-class-name": "agentgateway",
	"gateway.networking.k8s.io/gateway-name":       "gw",
	"agentgateway":                                 "kube-gateway",
}

func TestCreatePodDisruptionBudget_InheritsDeploymentLabels(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)

	pdb := createPodDisruptionBudget(gatewayWorkloadFromDeployment(dep))
	assert.Equal(t, gatewayLabels, pdb.GetLabels())
}

func TestPodDisruptionBudget_OverlayLabelsMergeOnTop(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &agentgateway.KubernetesResourceOverlay{
		Metadata: &agentgateway.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}
	pdb := createPodDisruptionBudget(gatewayWorkloadFromDeployment(dep))

	obj, err := applyOverlay(
		pdb,
		overlay,
		objectGVK(pdb),
	)
	require.NoError(t, err)

	pdb = obj.(*policyv1.PodDisruptionBudget)
	assert.Equal(t, "label", pdb.GetLabels()["extra"])
	for k, v := range gatewayLabels {
		assert.Equal(t, v, pdb.GetLabels()[k])
	}
}

func TestCreatePodDisruptionBudget_UsesDaemonSetSelector(t *testing.T) {
	daemonSet := &appsv1.DaemonSet{
		TypeMeta: metav1.TypeMeta{APIVersion: "apps/v1", Kind: "DaemonSet"},
		ObjectMeta: metav1.ObjectMeta{
			Name:      "test-daemonset",
			Namespace: "default",
			Labels:    gatewayLabels,
		},
		Spec: appsv1.DaemonSetSpec{
			Selector: &metav1.LabelSelector{MatchLabels: gatewayLabels},
		},
	}

	pdb := createPodDisruptionBudget(gatewayWorkloadFromDaemonSet(daemonSet))
	assert.Equal(t, daemonSet.Spec.Selector, pdb.Spec.Selector)
	assert.Equal(t, gatewayLabels, pdb.GetLabels())
}

func TestCreateHorizontalPodAutoscaler_InheritsDeploymentLabels(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)

	hpa := createHorizontalPodAutoscaler(gatewayWorkloadFromDeployment(dep))
	assert.Equal(t, gatewayLabels, hpa.GetLabels())
}

func TestHorizontalPodAutoscaler_OverlayLabelsMergeOnTop(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &agentgateway.KubernetesResourceOverlay{
		Metadata: &agentgateway.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}
	hpa := createHorizontalPodAutoscaler(gatewayWorkloadFromDeployment(dep))

	obj, err := applyOverlay(
		hpa,
		overlay,
		objectGVK(hpa),
	)
	require.NoError(t, err)

	hpa = obj.(*autoscalingv2.HorizontalPodAutoscaler)
	assert.Equal(t, "label", hpa.GetLabels()["extra"])
	for k, v := range gatewayLabels {
		assert.Equal(t, v, hpa.GetLabels()[k])
	}
}

func TestCreateVerticalPodAutoscaler_InheritsDeploymentLabels(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)

	vpa := createVerticalPodAutoscaler(gatewayWorkloadFromDeployment(dep))
	assert.Equal(t, gatewayLabels, vpa.GetLabels())
}

func TestVerticalPodAutoscaler_OverlayLabelsMergeOnTop(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &agentgateway.KubernetesResourceOverlay{
		Metadata: &agentgateway.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}

	vpa, err := applyVerticalPodAutoscalerOverlay(
		createVerticalPodAutoscaler(gatewayWorkloadFromDeployment(dep)),
		overlay,
	)
	require.NoError(t, err)

	assert.Equal(t, "label", vpa.GetLabels()["extra"])
	for k, v := range gatewayLabels {
		assert.Equal(t, v, vpa.GetLabels()[k])
	}
}

func TestCreatePodDisruptionBudget_ClonesDeploymentLabels(t *testing.T) {
	dep := deploymentWithLabels(gatewayLabels)
	overlay := &agentgateway.KubernetesResourceOverlay{
		Metadata: &agentgateway.ObjectMetadata{
			Labels: map[string]string{"extra": "label"},
		},
	}
	pdb := createPodDisruptionBudget(gatewayWorkloadFromDeployment(dep))

	_, err := applyOverlay(
		pdb,
		overlay,
		objectGVK(pdb),
	)
	require.NoError(t, err)

	// The original deployment labels must not have been mutated.
	assert.NotContains(t, dep.GetLabels(), "extra")
}

func podDisruptionBudgetsFromObjects(objs []client.Object) []*policyv1.PodDisruptionBudget {
	pdbs := make([]*policyv1.PodDisruptionBudget, 0)
	for _, obj := range objs {
		pdb, ok := obj.(*policyv1.PodDisruptionBudget)
		if ok {
			pdbs = append(pdbs, pdb)
		}
	}
	return pdbs
}

func horizontalPodAutoscalersFromObjects(objs []client.Object) []*autoscalingv2.HorizontalPodAutoscaler {
	hpas := make([]*autoscalingv2.HorizontalPodAutoscaler, 0)
	for _, obj := range objs {
		hpa, ok := obj.(*autoscalingv2.HorizontalPodAutoscaler)
		if ok {
			hpas = append(hpas, hpa)
		}
	}
	return hpas
}
