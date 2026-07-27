package cachetransform

import (
	"strings"
	"testing"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

// fullPodFixture returns a Pod populated with every field the controller
// might touch (and a bunch it doesn't) so the transform test can assert
// exactly what is kept and what is dropped.
func fullPodFixture() *corev1.Pod {
	return &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name:        "pod-1",
			Namespace:   "ns",
			Labels:      map[string]string{"app": "web"},
			Annotations: map[string]string{"note": "keep-me"},
			Finalizers:  []string{"test-finalizer"},
			ManagedFields: []metav1.ManagedFieldsEntry{{
				Manager: "kubelet", Operation: metav1.ManagedFieldsOperationApply,
			}},
		},
		Spec: corev1.PodSpec{
			NodeName:           "node-7",
			ServiceAccountName: "sa-1",
			HostNetwork:        true,
			Hostname:           "host-1",
			Subdomain:          "sub-1",
			Containers: []corev1.Container{{
				Name:  "c",
				Image: "debian:latest",
				Ports: []corev1.ContainerPort{{ContainerPort: 8080}},
				Env:   []corev1.EnvVar{{Name: "A", Value: "B"}},
			}},
			InitContainers: []corev1.Container{{Name: "init", Image: "busybox"}},
			Volumes:        []corev1.Volume{{Name: "v"}},
			Tolerations:    []corev1.Toleration{{Key: "k"}},
		},
		Status: corev1.PodStatus{
			Phase:  corev1.PodRunning,
			PodIP:  "10.0.0.1",
			PodIPs: []corev1.PodIP{{IP: "10.0.0.1"}},
			Conditions: []corev1.PodCondition{{
				Type: corev1.PodReady, Status: corev1.ConditionTrue,
			}},
			ContainerStatuses: []corev1.ContainerStatus{{
				Name: "c", Ready: true,
			}},
		},
	}
}

// TestPodCacheTransform verifies what the Pod transform keeps and drops.
// It layers on top of istio's StripPodUnusedFields, so the spec assertions
// mirror what istio already strips — the agentgateway-specific addition is
// that finalizers are also dropped.
func TestPodCacheTransform(t *testing.T) {
	out, err := PodCacheTransform(fullPodFixture())
	if err != nil {
		t.Fatalf("PodCacheTransform error: %v", err)
	}
	pod, ok := out.(*corev1.Pod)
	if !ok {
		t.Fatalf("transform returned %T, want *corev1.Pod", out)
	}

	// Dropped (agentgateway-specific).
	if pod.Finalizers != nil {
		t.Errorf("finalizers not stripped: %v", pod.Finalizers)
	}

	// Dropped (istio default — asserting here for documentation).
	if pod.ManagedFields != nil {
		t.Error("managedFields not stripped")
	}
	if len(pod.Spec.Containers) != 1 || len(pod.Spec.Containers[0].Env) != 0 {
		t.Errorf("spec.env leaked through istio transform: %+v", pod.Spec.Containers)
	}
	if len(pod.Spec.InitContainers) != 0 {
		t.Errorf("init containers not stripped by istio transform: %+v", pod.Spec.InitContainers)
	}
	if len(pod.Spec.Volumes) != 0 {
		t.Errorf("volumes not stripped: %+v", pod.Spec.Volumes)
	}
	if len(pod.Spec.Tolerations) != 0 {
		t.Errorf("tolerations not stripped: %+v", pod.Spec.Tolerations)
	}

	// Kept: the fields the controllers actually read.
	if pod.Spec.NodeName != "node-7" {
		t.Errorf("spec.nodeName lost: %q", pod.Spec.NodeName)
	}
	if pod.Spec.ServiceAccountName != "sa-1" {
		t.Errorf("spec.serviceAccountName lost: %q", pod.Spec.ServiceAccountName)
	}
	if !pod.Spec.HostNetwork {
		t.Error("spec.hostNetwork lost")
	}
	if pod.Labels["app"] != "web" {
		t.Error("labels lost")
	}
	if pod.Annotations["note"] != "keep-me" {
		t.Error("annotations lost")
	}
	if pod.Status.Phase != corev1.PodRunning || pod.Status.PodIP != "10.0.0.1" {
		t.Errorf("status lost: %+v", pod.Status)
	}
}

// TestPodCacheTransformMergePatchUnaffected proves the transform's safety
// claim: merge patches computed against a transform-stripped cache pod are
// byte-identical to those computed against the full pod, because the patch is
// a diff of the controller's own metadata mutations against a DeepCopy of the
// same cached base — stripped fields appear on neither side, so they can
// neither leak into nor be deleted by the patch.
func TestPodCacheTransformMergePatchUnaffected(t *testing.T) {
	// Metadata-only mutation of the kind the controllers perform: add a
	// label, drop an annotation.
	mutate := func(p *corev1.Pod) {
		p.Labels["example.com/extra"] = "v"
		delete(p.Annotations, "note")
	}

	// Patch from the FULL pod (behavior without the cache transform).
	full := fullPodFixture()
	fullBase := client.MergeFrom(full.DeepCopy())
	mutate(full)
	fullData, err := fullBase.Data(full)
	if err != nil {
		t.Fatalf("full-pod patch data: %v", err)
	}

	// Patch from the TRANSFORMED pod (behavior with the cache transform).
	strippedAny, err := PodCacheTransform(fullPodFixture())
	if err != nil {
		t.Fatalf("transform: %v", err)
	}
	stripped := strippedAny.(*corev1.Pod)
	strippedBase := client.MergeFrom(stripped.DeepCopy())
	mutate(stripped)
	strippedData, err := strippedBase.Data(stripped)
	if err != nil {
		t.Fatalf("stripped-pod patch data: %v", err)
	}

	if string(fullData) != string(strippedData) {
		t.Errorf("merge patch changed by cache transform:\n full:     %s\n stripped: %s", fullData, strippedData)
	}
	if strings.Contains(string(strippedData), "managedFields") {
		t.Errorf("patch touches managedFields: %s", strippedData)
	}
	if strings.Contains(string(strippedData), "finalizers") {
		t.Errorf("patch touches finalizers: %s", strippedData)
	}
}

// TestDefaultCacheTransform verifies that the default transform strips
// managedFields from arbitrary objects and leaves everything else intact.
func TestDefaultCacheTransform(t *testing.T) {
	cm := &corev1.ConfigMap{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "cm",
			Namespace: "ns",
			Labels:    map[string]string{"k": "v"},
			ManagedFields: []metav1.ManagedFieldsEntry{{
				Manager: "kubelet", Operation: metav1.ManagedFieldsOperationApply,
			}},
		},
		Data: map[string]string{"key": "value"},
	}

	out, err := DefaultCacheTransform(cm)
	if err != nil {
		t.Fatalf("DefaultCacheTransform error: %v", err)
	}
	got, ok := out.(*corev1.ConfigMap)
	if !ok {
		t.Fatalf("transform returned %T, want *corev1.ConfigMap", out)
	}
	if len(got.ManagedFields) != 0 {
		t.Errorf("managedFields not stripped: %v", got.ManagedFields)
	}
	if got.Labels["k"] != "v" {
		t.Error("labels lost")
	}
	if got.Data["key"] != "value" {
		t.Error("data lost")
	}
}

// TestPodCacheTransformNonPodPassthrough verifies that non-Pod objects pass
// through PodCacheTransform unchanged (important because the transform is
// wired into a shared informer factory that may pass tombstones or other
// object types through it).
func TestPodCacheTransformNonPodPassthrough(t *testing.T) {
	svc := &corev1.Service{ObjectMeta: metav1.ObjectMeta{Name: "s"}}
	got, err := PodCacheTransform(svc)
	if err != nil {
		t.Fatalf("transform of non-pod errored: %v", err)
	}
	if got != any(svc) {
		t.Error("non-pod object was not passed through unchanged")
	}

	// Arbitrary non-pod value must also pass through.
	cm := &corev1.ConfigMap{ObjectMeta: metav1.ObjectMeta{Name: "cm"}}
	got2, err := PodCacheTransform(cm)
	if err != nil {
		t.Fatalf("transform of ConfigMap errored: %v", err)
	}
	if got2 != any(cm) {
		t.Error("ConfigMap was not passed through unchanged")
	}
}

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
	if got.DeletionTimestamp != nil {
		t.Errorf("deletionTimestamp not stripped: %v", got.DeletionTimestamp)
	}
	if got.DeletionGracePeriodSeconds != nil {
		t.Errorf("deletionGracePeriodSeconds not stripped: %v", got.DeletionGracePeriodSeconds)
	}
	if got.Generation != 0 {
		t.Errorf("generation not stripped: %d", got.Generation)
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
