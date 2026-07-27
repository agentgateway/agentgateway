// Package cachetransform provides informer cache transform functions that
// strip unused fields from Kubernetes objects before they are stored in the
// controller's informer caches. This reduces both cache memory footprint and
// per-event JSON decode cost, keeping them proportional to the fields the
// controllers actually read.
package cachetransform

import (
	istiokube "istio.io/istio/pkg/kube"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// PodCacheTransform is a client-go informer transform for the manager's Pod
// cache. It layers on top of istio's StripPodUnusedFields (which already
// drops managedFields, the proxyOverrides annotation, and most of the pod
// spec) by additionally stripping metadata.finalizers.
//
// Why this is safe:
//   - No controller in this repo writes Pod finalizers; the controller's
//     RBAC finalizer rule applies to agentgateway CRs, not Pods.
//   - The pod spec the controller writes is built from the Gateway spec /
//     Helm values, never from the cached pod. All pod writes in this repo
//     target metadata only, diffed against a DeepCopy of the same
//     transformed cache object. Stripped fields appear on neither side of
//     the diff, so they can neither leak into nor be deleted by a patch.
//     See TestPodCacheTransformMergePatchUnaffected.
//   - istio's ambient mesh WorkloadsCollection / WaypointsCollection (the
//     downstream consumers of this Pod collection) do not read finalizers.
//
// Non-pod inputs (e.g. cache.DeletedFinalStateUnknown tombstones) pass
// through unchanged.
func PodCacheTransform(obj any) (any, error) {
	pod, ok := obj.(*corev1.Pod)
	if !ok {
		// istio's StripPodUnusedFields panics on non-Pod objects because
		// it performs an unchecked *Pod type assertion. Guard it here so
		// the transform is safe to use as a default for mixed-type caches.
		return obj, nil
	}
	// Apply istio's standard Pod transform first (strips managedFields,
	// proxyOverrides annotation, and most of the pod spec — keeping only
	// container ports, serviceAccountName, nodeName, hostNetwork, hostname,
	// subdomain).
	obj, err := istiokube.StripPodUnusedFields(pod)
	if err != nil {
		return nil, err
	}
	pod = obj.(*corev1.Pod)
	// Drop finalizers: no controller in this repo reads or writes Pod
	// finalizers; keeping them is pure cache memory waste.
	pod.Finalizers = nil
	return pod, nil
}

// DefaultCacheTransform strips metadata.managedFields from every cached
// object. This is the same transform istio's informer factory installs by
// default (stripUnusedFields), provided here for explicit use in the
// controller-runtime cache options where istio's default does not apply.
//
// Nothing in this repo reads managedFields; every write is either a merge
// patch diffed between two equally-stripped copies (managedFields can never
// appear in the diff) or an update/create, where absent managedFields means
// "leave server-side field management unchanged". Pure decode-CPU/memory win.
//
// Note: this intentionally does NOT strip finalizers from arbitrary objects.
// Finalizers can affect merge-patch correctness — if a controller sets or
// clears a finalizer, the diff must reflect that. Only Pod finalizers are
// stripped, because no controller in this repo touches them (see
// PodCacheTransform for the safety argument).
func DefaultCacheTransform(obj any) (any, error) {
	t, ok := obj.(metav1.ObjectMetaAccessor)
	if !ok {
		return obj, nil
	}
	t.GetObjectMeta().SetManagedFields(nil)
	return obj, nil
}

// ServiceCacheTransform is a client-go informer transform for the manager's
// Service cache. It strips fields the controllers and istio ambient mesh
// builders never read, before the object is stored, so cache memory and
// per-event JSON decode cost stay proportional to what is actually used.
//
// Stripped (conservative — only fields proven safe to drop):
//   - metadata.managedFields: written via server-side apply by every
//     controller that touches the Service; never read here.
//   - metadata.creationTimestamp: no controller reads creation time.
//   - metadata.deletionTimestamp: no controller reads this on Services.
//   - metadata.deletionGracePeriodSeconds: no controller reads this.
//   - metadata.generation: Services don't use the generation/observedGeneration
//     reconciliation pattern; no controller reads it.
//
// Intentionally preserved:
//   - metadata.finalizers: stripping finalizers breaks merge-patch
//     correctness (verified by TestServiceCacheTransformFinalizersAffectPatch).
//     When a controller computes a merge patch against a cache object whose
//     finalizers were stripped, the resulting patch may differ from one
//     computed against the full object.
//   - metadata (labels, annotations, ownerReferences) — used for ownership
//     checks and label-based status propagation.
//   - spec.type / spec.clusterIP(s) / spec.ports / spec.selector — read by
//     the gateway reconciler for status addresses and by istio ambient
//     mesh builders for service discovery.
//   - status.loadBalancer — read for Gateway status addresses.
//
// Non-Service inputs pass through untouched.
func ServiceCacheTransform(obj any) (any, error) {
	svc, ok := obj.(*corev1.Service)
	if !ok {
		return obj, nil
	}
	svc.ManagedFields = nil
	svc.CreationTimestamp = metav1.Time{}
	svc.DeletionTimestamp = nil
	svc.DeletionGracePeriodSeconds = nil
	svc.Generation = 0
	return svc, nil
}
