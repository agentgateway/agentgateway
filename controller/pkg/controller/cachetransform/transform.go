// Package cachetransform provides informer cache transform functions that
// strip unused fields from Kubernetes objects before they are stored in the
// controller's informer caches. This reduces both cache memory footprint and
// per-event JSON decode cost, keeping them proportional to the fields the
// controllers actually read.
package cachetransform

import (
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

// ServiceCacheTransform is a client-go informer transform for the manager's
// Service cache. It strips fields the controllers and istio ambient mesh
// builders never read, before the object is stored, so cache memory and
// per-event JSON decode cost stay proportional to what is actually used.
//
// Stripped (conservative — only fields proven safe to drop):
//   - metadata.managedFields: written via server-side apply by every
//     controller that touches the Service; never read here.
//   - metadata.creationTimestamp: no controller reads creation time.
//   - metadata.deletionGracePeriodSeconds: no controller reads this.
//   - metadata.generation: Services don't use the generation/observedGeneration
//     reconciliation pattern; no controller reads it.
//
// Intentionally preserved:
//   - metadata.deletionTimestamp: read by istio's ambient mesh code to
//     determine object lifecycle state (e.g. workloads.go: IsPodReady ||
//     DeletionTimestamp != nil).
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
	svc.DeletionGracePeriodSeconds = nil
	svc.Generation = 0
	return svc, nil
}
