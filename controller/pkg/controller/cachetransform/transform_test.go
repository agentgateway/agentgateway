package cachetransform

import (
	"strings"
	"testing"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

// fullServiceFixture returns a Service populated with every field the
// controller and istio ambient mesh builders might touch.
func fullServiceFixture() *corev1.Service {
	now := metav1.Now()
	grace := int64(30)
	return &corev1.Service{
		ObjectMeta: metav1.ObjectMeta{
			Name:                       "svc-1",
			Namespace:                  "ns",
			Labels:                     map[string]string{"app": "web"},
			Annotations:                map[string]string{"note": "keep-me"},
			Finalizers:                 []string{"service-finalizer"},
			Generation:                 42,
			CreationTimestamp:          now,
			DeletionTimestamp:          &now,
			DeletionGracePeriodSeconds: &grace,
			OwnerReferences: []metav1.OwnerReference{{
				APIVersion: "gateway.networking.k8s.io/v1", Kind: "Gateway", Name: "gw-1",
			}},
			ManagedFields: []metav1.ManagedFieldsEntry{{
				Manager: "controller", Operation: metav1.ManagedFieldsOperationApply,
			}},
		},
		Spec: corev1.ServiceSpec{
			Type:       corev1.ServiceTypeLoadBalancer,
			ClusterIP:  "10.0.0.1",
			ClusterIPs: []string{"10.0.0.1"},
			Ports:      []corev1.ServicePort{{Port: 80}},
			Selector:   map[string]string{"app": "web"},
		},
		Status: corev1.ServiceStatus{
			LoadBalancer: corev1.LoadBalancerStatus{
				Ingress: []corev1.LoadBalancerIngress{{IP: "203.0.113.1"}},
			},
		},
	}
}

// TestServiceCacheTransform verifies what the conservative Service transform
// keeps and drops.
func TestServiceCacheTransform(t *testing.T) {
	out, err := ServiceCacheTransform(fullServiceFixture())
	if err != nil {
		t.Fatalf("ServiceCacheTransform error: %v", err)
	}
	got, ok := out.(*corev1.Service)
	if !ok {
		t.Fatalf("transform returned %T, want *corev1.Service", out)
	}

	// Dropped.
	if got.ManagedFields != nil {
		t.Error("managedFields not stripped")
	}
	if !got.CreationTimestamp.IsZero() {
		t.Errorf("creationTimestamp not stripped: %v", got.CreationTimestamp)
	}
	if got.DeletionGracePeriodSeconds != nil {
		t.Errorf("deletionGracePeriodSeconds not stripped: %v", got.DeletionGracePeriodSeconds)
	}
	if got.Generation != 0 {
		t.Errorf("generation not stripped: %d", got.Generation)
	}

	// Intentionally preserved: deletionTimestamp (read by istio ambient).
	if got.DeletionTimestamp == nil {
		t.Error("deletionTimestamp unexpectedly stripped; istio ambient reads it")
	}

	// Intentionally preserved: finalizers (stripping would break merge-patch).
	if len(got.Finalizers) != 1 || got.Finalizers[0] != "service-finalizer" {
		t.Errorf("finalizers unexpectedly stripped: %v", got.Finalizers)
	}

	// Preserved: metadata the controller reads.
	if got.Labels["app"] != "web" {
		t.Error("labels lost")
	}
	if got.Annotations["note"] != "keep-me" {
		t.Error("annotations lost")
	}
	if len(got.OwnerReferences) != 1 {
		t.Errorf("ownerReferences lost: %+v", got.OwnerReferences)
	}

	// Preserved: spec fields the controller and istio ambient mesh read.
	if got.Spec.Type != corev1.ServiceTypeLoadBalancer {
		t.Errorf("spec.type lost: %q", got.Spec.Type)
	}
	if got.Spec.ClusterIP != "10.0.0.1" {
		t.Errorf("spec.clusterIP lost: %q", got.Spec.ClusterIP)
	}
	if len(got.Spec.Ports) != 1 {
		t.Errorf("spec.ports lost: %+v", got.Spec.Ports)
	}
	if got.Spec.Selector["app"] != "web" {
		t.Error("spec.selector lost")
	}

	// Preserved: status.loadBalancer (read by gw_controller for Gateway status).
	if len(got.Status.LoadBalancer.Ingress) != 1 {
		t.Errorf("status.loadBalancer lost: %+v", got.Status.LoadBalancer)
	}
}

// TestServiceCacheTransformMergePatchUnaffected proves the transform's safety
// claim: metadata-only merge patches computed against a transform-stripped
// cache Service are byte-identical to those computed against the full Service.
func TestServiceCacheTransformMergePatchUnaffected(t *testing.T) {
	// Metadata-only mutation: add a label, drop an annotation.
	mutate := func(s *corev1.Service) {
		s.Labels["example.com/extra"] = "v"
		delete(s.Annotations, "note")
	}

	// Patch from the FULL service.
	full := fullServiceFixture()
	fullBase := client.MergeFrom(full.DeepCopy())
	mutate(full)
	fullData, err := fullBase.Data(full)
	if err != nil {
		t.Fatalf("full-svc patch data: %v", err)
	}

	// Patch from the TRANSFORMED service.
	strippedAny, err := ServiceCacheTransform(fullServiceFixture())
	if err != nil {
		t.Fatalf("transform: %v", err)
	}
	stripped := strippedAny.(*corev1.Service)
	strippedBase := client.MergeFrom(stripped.DeepCopy())
	mutate(stripped)
	strippedData, err := strippedBase.Data(stripped)
	if err != nil {
		t.Fatalf("stripped-svc patch data: %v", err)
	}

	if string(fullData) != string(strippedData) {
		t.Errorf("merge patch changed by cache transform:\n full:     %s\n stripped: %s", fullData, strippedData)
	}
	if strings.Contains(string(strippedData), "managedFields") {
		t.Errorf("patch touches managedFields: %s", strippedData)
	}
	if strings.Contains(string(strippedData), "creationTimestamp") {
		t.Errorf("patch touches creationTimestamp: %s", strippedData)
	}
}

// TestServiceCacheTransformFinalizersAffectPatch documents WHY we don't strip
// Service finalizers: doing so changes the merge patch output. This test
// uses a mutation that sets finalizers to demonstrate the divergence.
func TestServiceCacheTransformFinalizersAffectPatch(t *testing.T) {
	// Mutation: set a finalizer (a plausible controller write).
	mutate := func(s *corev1.Service) {
		s.Finalizers = append(s.Finalizers, "new-finalizer")
	}

	// Patch from the FULL service.
	full := fullServiceFixture()
	fullBase := client.MergeFrom(full.DeepCopy())
	mutate(full)
	fullData, err := fullBase.Data(full)
	if err != nil {
		t.Fatalf("full-svc patch data: %v", err)
	}

	// Patch from the TRANSFORMED service (finalizers preserved by our
	// conservative transform).
	strippedAny, _ := ServiceCacheTransform(fullServiceFixture())
	stripped := strippedAny.(*corev1.Service)
	strippedBase := client.MergeFrom(stripped.DeepCopy())
	mutate(stripped)
	strippedData, err := strippedBase.Data(stripped)
	if err != nil {
		t.Fatalf("stripped-svc patch data: %v", err)
	}

	// Patches MUST be identical: our transform preserves finalizers.
	if string(fullData) != string(strippedData) {
		t.Errorf("conservative transform should preserve merge-patch correctness for finalizer mutations:\n full:     %s\n stripped: %s", fullData, strippedData)
	}
}

// TestServiceCacheTransformNonServicePassthrough verifies non-Service objects
// pass through ServiceCacheTransform unchanged.
func TestServiceCacheTransformNonServicePassthrough(t *testing.T) {
	pod := &corev1.Pod{ObjectMeta: metav1.ObjectMeta{Name: "p"}}
	got, err := ServiceCacheTransform(pod)
	if err != nil {
		t.Fatalf("transform of non-Service errored: %v", err)
	}
	if got != any(pod) {
		t.Error("non-Service object was not passed through unchanged")
	}
}
