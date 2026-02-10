package strategicpatch

import (
	"encoding/json"
	"fmt"
	"maps"

	appsv1 "k8s.io/api/apps/v1"
	autoscalingv2 "k8s.io/api/autoscaling/v2"
	corev1 "k8s.io/api/core/v1"
	policyv1 "k8s.io/api/policy/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/util/strategicpatch"
	"sigs.k8s.io/controller-runtime/pkg/client"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/shared"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
)

// ResourceOverlays contains all the overlays that can be applied to rendered objects.
type ResourceOverlays struct {
	Deployment              *shared.KubernetesResourceOverlay
	Service                 *shared.KubernetesResourceOverlay
	ServiceAccount          *shared.KubernetesResourceOverlay
	PodDisruptionBudget     *shared.KubernetesResourceOverlay
	HorizontalPodAutoscaler *shared.KubernetesResourceOverlay
	VerticalPodAutoscaler   *shared.KubernetesResourceOverlay
}

// FromAgentgatewayParameters converts AgentgatewayParameters overlays to generic ResourceOverlays.
func FromAgentgatewayParameters(params *agentgateway.AgentgatewayParameters) *ResourceOverlays {
	if params == nil {
		return nil
	}
	overlays := params.Spec.AgentgatewayParametersOverlays
	return &ResourceOverlays{
		Deployment:              overlays.Deployment,
		Service:                 overlays.Service,
		ServiceAccount:          overlays.ServiceAccount,
		PodDisruptionBudget:     overlays.PodDisruptionBudget,
		HorizontalPodAutoscaler: overlays.HorizontalPodAutoscaler,
		// AgentgatewayParameters does not have VPA support
		VerticalPodAutoscaler: nil,
	}
}

// OverlayApplier applies overlays to rendered k8s objects using strategic merge patch semantics.
type OverlayApplier struct {
	overlays *ResourceOverlays
}

// NewOverlayApplier creates a new OverlayApplier from AgentgatewayParameters.
func NewOverlayApplier(params *agentgateway.AgentgatewayParameters) *OverlayApplier {
	return &OverlayApplier{overlays: FromAgentgatewayParameters(params)}
}

// ApplyOverlays applies the overlays to the rendered objects.
// It modifies the objects in place and may append new objects (PDB, HPA, VPA) to the slice.
// The caller must use the returned slice as the objects list may grow.
func (a *OverlayApplier) ApplyOverlays(objs []client.Object) ([]client.Object, error) {
	if a.overlays == nil {
		return objs, nil
	}

	// Find the Deployment first - we need it for PDB/HPA/VPA creation
	var deployment *appsv1.Deployment
	for _, obj := range objs {
		if dep, ok := obj.(*appsv1.Deployment); ok {
			deployment = dep
			break
		}
	}

	for i, obj := range objs {
		var overlay *shared.KubernetesResourceOverlay
		var gvk schema.GroupVersionKind

		// Use type assertions to determine the object type, as GVK may not be set
		// on typed structs rendered from Helm charts
		switch obj.(type) {
		case *appsv1.Deployment:
			overlay = a.overlays.Deployment
			gvk = wellknown.DeploymentGVK
		case *corev1.Service:
			overlay = a.overlays.Service
			gvk = wellknown.ServiceGVK
		case *corev1.ServiceAccount:
			overlay = a.overlays.ServiceAccount
			gvk = wellknown.ServiceAccountGVK
		default:
			continue
		}

		if overlay == nil {
			continue
		}

		patched, err := applyOverlay(obj, overlay, gvk)
		if err != nil {
			return nil, fmt.Errorf("failed to apply overlay to %s/%s: %w", gvk.Kind, obj.GetName(), err)
		}
		objs[i] = patched
	}

	// Create PDB if overlay is present
	if a.overlays.PodDisruptionBudget != nil && deployment != nil {
		pdb, err := createPodDisruptionBudget(deployment, a.overlays.PodDisruptionBudget)
		if err != nil {
			return nil, fmt.Errorf("failed to create PodDisruptionBudget: %w", err)
		}
		objs = append(objs, pdb)
	}

	// Create HPA if overlay is present
	if a.overlays.HorizontalPodAutoscaler != nil && deployment != nil {
		hpa, err := createHorizontalPodAutoscaler(deployment, a.overlays.HorizontalPodAutoscaler)
		if err != nil {
			return nil, fmt.Errorf("failed to create HorizontalPodAutoscaler: %w", err)
		}
		objs = append(objs, hpa)
	}

	// Create VPA if overlay is present
	if a.overlays.VerticalPodAutoscaler != nil && deployment != nil {
		vpa, err := createVerticalPodAutoscaler(deployment, a.overlays.VerticalPodAutoscaler)
		if err != nil {
			return nil, fmt.Errorf("failed to create VerticalPodAutoscaler: %w", err)
		}
		objs = append(objs, vpa)
	}

	return objs, nil
}

// applyOverlay applies a KubernetesResourceOverlay to a single object.
func applyOverlay(obj client.Object, overlay *shared.KubernetesResourceOverlay, gvk schema.GroupVersionKind) (client.Object, error) {
	// Apply metadata first
	if overlay.Metadata != nil {
		if overlay.Metadata.Labels != nil {
			existingLabels := obj.GetLabels()
			if existingLabels == nil {
				existingLabels = make(map[string]string)
			}
			maps.Copy(existingLabels, overlay.Metadata.Labels)
			obj.SetLabels(existingLabels)
		}
		if overlay.Metadata.Annotations != nil {
			existingAnnotations := obj.GetAnnotations()
			if existingAnnotations == nil {
				existingAnnotations = make(map[string]string)
			}
			maps.Copy(existingAnnotations, overlay.Metadata.Annotations)
			obj.SetAnnotations(existingAnnotations)
		}
	}

	// Apply spec overlay using strategic merge patch if present
	if overlay.Spec != nil && len(overlay.Spec.Raw) > 0 {
		return applySpecOverlay(obj, overlay.Spec.Raw, gvk)
	}

	return obj, nil
}

// applySpecOverlay applies a spec overlay using strategic merge patch semantics.
func applySpecOverlay(obj client.Object, patchBytes []byte, gvk schema.GroupVersionKind) (client.Object, error) {
	// Get the schema for strategic merge patch
	dataObj, err := getDataObjectForGVK(gvk)
	if err != nil {
		return nil, fmt.Errorf("unsupported kind %s for strategic merge patch: %w", gvk.Kind, err)
	}

	// Serialize the original object to JSON
	originalBytes, err := json.Marshal(obj)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal original object: %w", err)
	}

	// The patch from the user is for the spec field, but strategic merge patch
	// expects the full object structure. Wrap the patch in a spec field.
	wrappedPatch := map[string]json.RawMessage{
		"spec": patchBytes,
	}
	wrappedPatchBytes, err := json.Marshal(wrappedPatch)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal wrapped patch: %w", err)
	}

	// Apply strategic merge patch
	patchedBytes, err := strategicpatch.StrategicMergePatch(originalBytes, wrappedPatchBytes, dataObj)
	if err != nil {
		return nil, fmt.Errorf("failed to apply strategic merge patch: %w", err)
	}

	// Deserialize back to the object
	patchedObj, err := deserializeToObject(patchedBytes, gvk)
	if err != nil {
		return nil, fmt.Errorf("failed to deserialize patched object: %w", err)
	}

	return patchedObj, nil
}

// getDataObjectForGVK returns an empty object of the appropriate type for strategic merge patch.
func getDataObjectForGVK(gvk schema.GroupVersionKind) (runtime.Object, error) {
	switch gvk.Kind {
	case wellknown.DeploymentGVK.Kind:
		return &appsv1.Deployment{}, nil
	case wellknown.ServiceGVK.Kind:
		return &corev1.Service{}, nil
	case wellknown.ServiceAccountGVK.Kind:
		return &corev1.ServiceAccount{}, nil
	case wellknown.PodDisruptionBudgetGVK.Kind:
		return &policyv1.PodDisruptionBudget{}, nil
	case wellknown.HorizontalPodAutoscalerGVK.Kind:
		return &autoscalingv2.HorizontalPodAutoscaler{}, nil
	case wellknown.VerticalPodAutoscalerGVK.Kind:
		// VPA is a CRD, use unstructured for strategic merge
		return &unstructured.Unstructured{}, nil
	default:
		return nil, fmt.Errorf("unsupported kind: %s", gvk.Kind)
	}
}

// deserializeToObject deserializes JSON bytes to a typed k8s object.
func deserializeToObject(data []byte, gvk schema.GroupVersionKind) (client.Object, error) {
	// For VPA, use unstructured since it's a CRD
	if gvk.Kind == wellknown.VerticalPodAutoscalerGVK.Kind {
		obj := &unstructured.Unstructured{}
		if err := json.Unmarshal(data, obj); err != nil {
			return nil, fmt.Errorf("failed to unmarshal patched object: %w", err)
		}
		obj.SetGroupVersionKind(gvk)
		return obj, nil
	}

	obj, err := getDataObjectForGVK(gvk)
	if err != nil {
		return nil, err
	}
	if err := json.Unmarshal(data, obj); err != nil {
		return nil, fmt.Errorf("failed to unmarshal patched object: %w", err)
	}

	// Ensure the GVK is set on the returned object
	clientObj := obj.(client.Object)
	clientObj.GetObjectKind().SetGroupVersionKind(gvk)

	return clientObj, nil
}

// createPodDisruptionBudget creates a PodDisruptionBudget for the given Deployment
// with the overlay applied.
func createPodDisruptionBudget(deployment *appsv1.Deployment, overlay *shared.KubernetesResourceOverlay) (client.Object, error) {
	// Create base PDB with selector matching the Deployment
	pdb := &policyv1.PodDisruptionBudget{
		TypeMeta: metav1.TypeMeta{
			APIVersion: wellknown.PodDisruptionBudgetGVK.GroupVersion().String(),
			Kind:       wellknown.PodDisruptionBudgetGVK.Kind,
		},
		ObjectMeta: metav1.ObjectMeta{
			Name:      deployment.Name,
			Namespace: deployment.Namespace,
		},
		Spec: policyv1.PodDisruptionBudgetSpec{
			Selector: deployment.Spec.Selector,
		},
	}

	// Apply the overlay
	patched, err := applyOverlay(pdb, overlay, wellknown.PodDisruptionBudgetGVK)
	if err != nil {
		return nil, err
	}

	return patched, nil
}

// createHorizontalPodAutoscaler creates a HorizontalPodAutoscaler for the given Deployment
// with the overlay applied.
func createHorizontalPodAutoscaler(deployment *appsv1.Deployment, overlay *shared.KubernetesResourceOverlay) (client.Object, error) {
	// Create base HPA with scaleTargetRef pointing to the Deployment
	hpa := &autoscalingv2.HorizontalPodAutoscaler{
		TypeMeta: metav1.TypeMeta{
			APIVersion: wellknown.HorizontalPodAutoscalerGVK.GroupVersion().String(),
			Kind:       wellknown.HorizontalPodAutoscalerGVK.Kind,
		},
		ObjectMeta: metav1.ObjectMeta{
			Name:      deployment.Name,
			Namespace: deployment.Namespace,
		},
		Spec: autoscalingv2.HorizontalPodAutoscalerSpec{
			ScaleTargetRef: autoscalingv2.CrossVersionObjectReference{
				APIVersion: wellknown.DeploymentGVK.GroupVersion().String(),
				Kind:       wellknown.DeploymentGVK.Kind,
				Name:       deployment.Name,
			},
		},
	}

	// Apply the overlay
	patched, err := applyOverlay(hpa, overlay, wellknown.HorizontalPodAutoscalerGVK)
	if err != nil {
		return nil, err
	}

	return patched, nil
}

// createVerticalPodAutoscaler creates a VerticalPodAutoscaler for the given Deployment
// with the overlay applied.
func createVerticalPodAutoscaler(deployment *appsv1.Deployment, overlay *shared.KubernetesResourceOverlay) (client.Object, error) {
	// Create base VPA with targetRef pointing to the Deployment
	// VPA is a CRD, so we use unstructured
	vpa := &unstructured.Unstructured{
		Object: map[string]any{
			"apiVersion": wellknown.VerticalPodAutoscalerGVK.GroupVersion().String(),
			"kind":       wellknown.VerticalPodAutoscalerGVK.Kind,
			"metadata": map[string]any{
				"name":      deployment.Name,
				"namespace": deployment.Namespace,
			},
			"spec": map[string]any{
				"targetRef": map[string]any{
					"apiVersion": wellknown.DeploymentGVK.GroupVersion().String(),
					"kind":       wellknown.DeploymentGVK.Kind,
					"name":       deployment.Name,
				},
			},
		},
	}
	vpa.SetGroupVersionKind(wellknown.VerticalPodAutoscalerGVK)

	// Apply the overlay - for VPA we need to handle it specially since it's unstructured
	if overlay.Metadata != nil {
		if overlay.Metadata.Labels != nil {
			existingLabels := vpa.GetLabels()
			if existingLabels == nil {
				existingLabels = make(map[string]string)
			}
			maps.Copy(existingLabels, overlay.Metadata.Labels)
			vpa.SetLabels(existingLabels)
		}
		if overlay.Metadata.Annotations != nil {
			existingAnnotations := vpa.GetAnnotations()
			if existingAnnotations == nil {
				existingAnnotations = make(map[string]string)
			}
			maps.Copy(existingAnnotations, overlay.Metadata.Annotations)
			vpa.SetAnnotations(existingAnnotations)
		}
	}

	// Apply spec overlay if present
	if overlay.Spec != nil && len(overlay.Spec.Raw) > 0 {
		// Parse the spec overlay
		var specPatch map[string]any
		if err := json.Unmarshal(overlay.Spec.Raw, &specPatch); err != nil {
			return nil, fmt.Errorf("failed to unmarshal spec patch: %w", err)
		}

		// Merge the spec patch into the VPA spec
		existingSpec, _, _ := unstructured.NestedMap(vpa.Object, "spec")
		if existingSpec == nil {
			existingSpec = make(map[string]any)
		}
		// Deep merge the patch into existing spec
		maps.Copy(existingSpec, specPatch)
		if err := unstructured.SetNestedMap(vpa.Object, existingSpec, "spec"); err != nil {
			return nil, fmt.Errorf("failed to set VPA spec: %w", err)
		}
	}

	return vpa, nil
}
