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

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

// ResourceOverlays contains all the overlays that can be applied to rendered objects.
type ResourceOverlays struct {
	Deployment              *agentgateway.KubernetesResourceOverlay
	DaemonSet               *agentgateway.KubernetesResourceOverlay
	Service                 *agentgateway.KubernetesResourceOverlay
	ServiceAccount          *agentgateway.KubernetesResourceOverlay
	PodDisruptionBudget     *agentgateway.KubernetesResourceOverlay
	HorizontalPodAutoscaler *agentgateway.KubernetesResourceOverlay
	VerticalPodAutoscaler   *agentgateway.KubernetesResourceOverlay
}

type gatewayWorkload struct {
	name      string
	namespace string
	labels    map[string]string
	selector  *metav1.LabelSelector
	gvk       schema.GroupVersionKind
}

func gatewayWorkloadFromDeployment(deployment *appsv1.Deployment) *gatewayWorkload {
	if deployment == nil {
		return nil
	}
	return &gatewayWorkload{
		name:      deployment.Name,
		namespace: deployment.Namespace,
		labels:    maps.Clone(deployment.GetLabels()),
		selector:  deployment.Spec.Selector,
		gvk:       wellknown.DeploymentGVK,
	}
}

func gatewayWorkloadFromDaemonSet(daemonSet *appsv1.DaemonSet) *gatewayWorkload {
	if daemonSet == nil {
		return nil
	}
	return &gatewayWorkload{
		name:      daemonSet.Name,
		namespace: daemonSet.Namespace,
		labels:    maps.Clone(daemonSet.GetLabels()),
		selector:  daemonSet.Spec.Selector,
		gvk:       wellknown.DaemonSetGVK,
	}
}

func gatewayWorkloadFromObjects(objs []client.Object) *gatewayWorkload {
	for _, obj := range objs {
		switch typed := obj.(type) {
		case *appsv1.Deployment:
			return gatewayWorkloadFromDeployment(typed)
		case *appsv1.DaemonSet:
			return gatewayWorkloadFromDaemonSet(typed)
		}
	}
	return nil
}

func (w *gatewayWorkload) targetRef() autoscalingv2.CrossVersionObjectReference {
	return autoscalingv2.CrossVersionObjectReference{
		APIVersion: w.gvk.GroupVersion().String(),
		Kind:       w.gvk.Kind,
		Name:       w.name,
	}
}

// FromAgentgatewayParameters converts AgentgatewayParameters overlays to generic ResourceOverlays.
func FromAgentgatewayParameters(params *agentgateway.AgentgatewayParameters) *ResourceOverlays {
	if params == nil {
		return nil
	}
	overlays := params.Spec.AgentgatewayParametersOverlays
	return &ResourceOverlays{
		Deployment:              overlays.Deployment,
		DaemonSet:               overlays.DaemonSet,
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

// LayeredOverlayApplier applies multiple overlay layers in precedence order.
type LayeredOverlayApplier struct {
	overlays []*ResourceOverlays
}

// NewOverlayApplier creates a new OverlayApplier from AgentgatewayParameters.
func NewOverlayApplier(params *agentgateway.AgentgatewayParameters) *OverlayApplier {
	return &OverlayApplier{overlays: FromAgentgatewayParameters(params)}
}

// NewLayeredOverlayApplier creates a new applier from ordered AgentgatewayParameters layers.
func NewLayeredOverlayApplier(params ...*agentgateway.AgentgatewayParameters) *LayeredOverlayApplier {
	overlays := make([]*ResourceOverlays, 0, len(params))
	for _, p := range params {
		if overlay := FromAgentgatewayParameters(p); overlay != nil {
			overlays = append(overlays, overlay)
		}
	}
	return &LayeredOverlayApplier{overlays: overlays}
}

// ApplyOverlays applies the overlays to the rendered objects.
// It modifies the objects in place and may append new objects (PDB, HPA, VPA) to the slice.
// The caller must use the returned slice as the objects list may grow.
func (a *OverlayApplier) ApplyOverlays(objs []client.Object) ([]client.Object, error) {
	return applyOverlayLayers(objs, a.overlays)
}

// ApplyOverlays applies the overlays to rendered objects in layer order.
func (a *LayeredOverlayApplier) ApplyOverlays(objs []client.Object) ([]client.Object, error) {
	if a == nil {
		return objs, nil
	}
	return applyOverlayLayers(objs, a.overlays...)
}

// applyOverlayLayers applies non-nil overlay layers in order and appends generated support resources.
func applyOverlayLayers(objs []client.Object, layers ...*ResourceOverlays) ([]client.Object, error) {
	if len(layers) == 0 {
		return objs, nil
	}

	filtered := make([]*ResourceOverlays, 0, len(layers))
	for _, l := range layers {
		if l != nil {
			filtered = append(filtered, l)
		}
	}
	if len(filtered) == 0 {
		return objs, nil
	}

	var err error
	for _, l := range filtered {
		objs, err = applyPrimaryOverlays(objs, l)
		if err != nil {
			return nil, err
		}
	}

	workload := gatewayWorkloadFromObjects(objs)
	objs, err = appendSupportResources(objs, workload, filtered)
	if err != nil {
		return nil, err
	}

	if err := ensureUniqueObjects(objs); err != nil {
		return nil, err
	}

	return objs, nil
}

func applyPrimaryOverlays(objs []client.Object, overlays *ResourceOverlays) ([]client.Object, error) {
	for i, obj := range objs {
		var overlay *agentgateway.KubernetesResourceOverlay
		var gvk schema.GroupVersionKind

		// Use type assertions to determine the object type, as GVK may not be set
		// on typed structs rendered from Helm charts
		switch obj.(type) {
		case *appsv1.Deployment:
			overlay = overlays.Deployment
			gvk = wellknown.DeploymentGVK
		case *appsv1.DaemonSet:
			overlay = overlays.DaemonSet
			gvk = wellknown.DaemonSetGVK
		case *corev1.Service:
			overlay = overlays.Service
			gvk = wellknown.ServiceGVK
		case *corev1.ServiceAccount:
			overlay = overlays.ServiceAccount
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
		objs[i] = patched //nolint:gosec // Safe: modifying slice element at current index during iteration
	}

	return objs, nil
}

func appendSupportResources(
	objs []client.Object,
	workload *gatewayWorkload,
	layers []*ResourceOverlays,
) ([]client.Object, error) {
	if workload == nil {
		return objs, nil
	}

	if hasPodDisruptionBudgetOverlay(layers) {
		pdb := client.Object(createPodDisruptionBudget(workload))
		for _, layer := range layers {
			if layer.PodDisruptionBudget == nil {
				continue
			}
			patched, err := applyOverlay(
				pdb,
				layer.PodDisruptionBudget,
				wellknown.PodDisruptionBudgetGVK,
			)
			if err != nil {
				return nil, fmt.Errorf("failed to apply PodDisruptionBudget overlay: %w", err)
			}
			pdb = patched
		}
		objs = append(objs, pdb)
	}

	if hasHorizontalPodAutoscalerOverlay(layers) {
		switch workload.gvk {
		case wellknown.DeploymentGVK:
			hpa := client.Object(createHorizontalPodAutoscaler(workload))
			for _, layer := range layers {
				if layer.HorizontalPodAutoscaler == nil {
					continue
				}
				patched, err := applyOverlay(
					hpa,
					layer.HorizontalPodAutoscaler,
					wellknown.HorizontalPodAutoscalerGVK,
				)
				if err != nil {
					return nil, fmt.Errorf("failed to apply HorizontalPodAutoscaler overlay: %w", err)
				}
				hpa = patched
			}
			objs = append(objs, hpa)
		default:
			return nil, fmt.Errorf(
				"horizontalPodAutoscaler overlay is not supported for %s workload",
				workload.gvk.Kind,
			)
		}
	}

	if hasVerticalPodAutoscalerOverlay(layers) && workload.gvk == wellknown.DeploymentGVK {
		vpa := createVerticalPodAutoscaler(workload)
		for _, layer := range layers {
			if layer.VerticalPodAutoscaler == nil {
				continue
			}
			patched, err := applyVerticalPodAutoscalerOverlay(vpa, layer.VerticalPodAutoscaler)
			if err != nil {
				return nil, fmt.Errorf("failed to apply VerticalPodAutoscaler overlay: %w", err)
			}
			vpa = patched
		}
		objs = append(objs, vpa)
	}

	return objs, nil
}

func hasPodDisruptionBudgetOverlay(layers []*ResourceOverlays) bool {
	for _, layer := range layers {
		if layer.PodDisruptionBudget != nil {
			return true
		}
	}
	return false
}

func hasHorizontalPodAutoscalerOverlay(layers []*ResourceOverlays) bool {
	for _, layer := range layers {
		if layer.HorizontalPodAutoscaler != nil {
			return true
		}
	}
	return false
}

func hasVerticalPodAutoscalerOverlay(layers []*ResourceOverlays) bool {
	for _, layer := range layers {
		if layer.VerticalPodAutoscaler != nil {
			return true
		}
	}
	return false
}

func ensureUniqueObjects(objs []client.Object) error {
	seen := make(map[corev1.ObjectReference]struct{}, len(objs))
	for _, obj := range objs {
		gvk := objectGVK(obj)
		if gvk.Empty() {
			continue
		}
		key := corev1.ObjectReference{
			APIVersion: gvk.GroupVersion().String(),
			Kind:       gvk.Kind,
			Namespace:  obj.GetNamespace(),
			Name:       obj.GetName(),
		}
		if _, exists := seen[key]; exists {
			return fmt.Errorf(
				"duplicate desired object %s %s/%s",
				gvk.String(),
				key.Namespace,
				key.Name,
			)
		}
		seen[key] = struct{}{}
	}
	return nil
}

func objectGVK(obj client.Object) schema.GroupVersionKind {
	gvk := obj.GetObjectKind().GroupVersionKind()
	if !gvk.Empty() {
		return gvk
	}

	switch obj.(type) {
	case *appsv1.Deployment:
		return wellknown.DeploymentGVK
	case *appsv1.DaemonSet:
		return wellknown.DaemonSetGVK
	case *corev1.Secret:
		return wellknown.SecretGVK
	case *corev1.ConfigMap:
		return wellknown.ConfigMapGVK
	case *corev1.Service:
		return wellknown.ServiceGVK
	case *corev1.ServiceAccount:
		return wellknown.ServiceAccountGVK
	case *policyv1.PodDisruptionBudget:
		return wellknown.PodDisruptionBudgetGVK
	case *autoscalingv2.HorizontalPodAutoscaler:
		return wellknown.HorizontalPodAutoscalerGVK
	case *unstructured.Unstructured:
		return obj.GetObjectKind().GroupVersionKind()
	default:
		return schema.GroupVersionKind{}
	}
}

// applyOverlay applies a KubernetesResourceOverlay to a single object.
func applyOverlay(obj client.Object, overlay *agentgateway.KubernetesResourceOverlay, gvk schema.GroupVersionKind) (client.Object, error) {
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
	case wellknown.DaemonSetGVK.Kind:
		return &appsv1.DaemonSet{}, nil
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

// createPodDisruptionBudget creates a PodDisruptionBudget for the selected workload
// with a selector matching the workload.
func createPodDisruptionBudget(workload *gatewayWorkload) *policyv1.PodDisruptionBudget {
	// Create base PDB with selector matching the workload
	pdb := &policyv1.PodDisruptionBudget{
		TypeMeta: metav1.TypeMeta{
			APIVersion: wellknown.PodDisruptionBudgetGVK.GroupVersion().String(),
			Kind:       wellknown.PodDisruptionBudgetGVK.Kind,
		},
		ObjectMeta: metav1.ObjectMeta{
			Name:      workload.name,
			Namespace: workload.namespace,
			Labels:    maps.Clone(workload.labels),
		},
		Spec: policyv1.PodDisruptionBudgetSpec{
			Selector: workload.selector,
		},
	}

	return pdb
}

// createHorizontalPodAutoscaler creates a HorizontalPodAutoscaler for the selected workload.
func createHorizontalPodAutoscaler(workload *gatewayWorkload) *autoscalingv2.HorizontalPodAutoscaler {
	// Create base HPA with scaleTargetRef pointing to the workload
	hpa := &autoscalingv2.HorizontalPodAutoscaler{
		TypeMeta: metav1.TypeMeta{
			APIVersion: wellknown.HorizontalPodAutoscalerGVK.GroupVersion().String(),
			Kind:       wellknown.HorizontalPodAutoscalerGVK.Kind,
		},
		ObjectMeta: metav1.ObjectMeta{
			Name:      workload.name,
			Namespace: workload.namespace,
			Labels:    maps.Clone(workload.labels),
		},
		Spec: autoscalingv2.HorizontalPodAutoscalerSpec{
			ScaleTargetRef: workload.targetRef(),
		},
	}

	return hpa
}

// createVerticalPodAutoscaler creates a VerticalPodAutoscaler for the selected workload.
func createVerticalPodAutoscaler(workload *gatewayWorkload) *unstructured.Unstructured {
	// Create base VPA with targetRef pointing to the Deployment
	// VPA is a CRD, so we use unstructured
	targetRef := workload.targetRef()
	vpa := &unstructured.Unstructured{
		Object: map[string]any{
			"apiVersion": wellknown.VerticalPodAutoscalerGVK.GroupVersion().String(),
			"kind":       wellknown.VerticalPodAutoscalerGVK.Kind,
			"metadata": map[string]any{
				"name":      workload.name,
				"namespace": workload.namespace,
			},
			"spec": map[string]any{
				"targetRef": map[string]any{
					"apiVersion": targetRef.APIVersion,
					"kind":       targetRef.Kind,
					"name":       targetRef.Name,
				},
			},
		},
	}
	vpa.SetGroupVersionKind(wellknown.VerticalPodAutoscalerGVK)
	vpa.SetLabels(maps.Clone(workload.labels))

	return vpa
}

func applyVerticalPodAutoscalerOverlay(
	vpa *unstructured.Unstructured,
	overlay *agentgateway.KubernetesResourceOverlay,
) (*unstructured.Unstructured, error) {
	if overlay == nil {
		return vpa, nil
	}

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
