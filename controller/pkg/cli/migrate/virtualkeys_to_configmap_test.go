package migrate

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"testing"
	"time"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/client-go/dynamic"
	dynamicfake "k8s.io/client-go/dynamic/fake"
	"k8s.io/client-go/kubernetes"
	k8sfake "k8s.io/client-go/kubernetes/fake"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gatewayapiclient "sigs.k8s.io/gateway-api/pkg/client/clientset/versioned"

	agentgateway "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	agentgatewayclient "github.com/agentgateway/agentgateway/controller/pkg/client/clientset/versioned"
)

// fakeCLIClient is a minimal kubeutil.CLIClient backed by fake clientsets.
type fakeCLIClient struct {
	kube kubernetes.Interface
	dyn  dynamic.Interface
}

func (f fakeCLIClient) Kube() kubernetes.Interface                 { return f.kube }
func (f fakeCLIClient) GatewayAPI() gatewayapiclient.Interface     { return nil }
func (f fakeCLIClient) Agentgateway() agentgatewayclient.Interface { return nil }
func (f fakeCLIClient) Dynamic() dynamic.Interface                 { return f.dyn }
func (f fakeCLIClient) AgentgatewayRequest(context.Context, string, string, string, string, int) ([]byte, error) {
	return nil, fmt.Errorf("not implemented")
}
func (f fakeCLIClient) NewPortForwarder(string, string, string, int, int) (kubeutil.PortForwarder, error) {
	return nil, fmt.Errorf("not implemented")
}

var virtualkeysPolicyGVR = schema.GroupVersionResource{Group: virtualkeysAPIGroup, Version: "v1alpha1", Resource: "agentgatewaypolicies"}

// virtualkeysFakeClient seeds discovery with agentgatewaypolicies and a dynamic client with policyObjects.
func virtualkeysFakeClient(t *testing.T, secret *corev1.Secret, policyObjects ...runtime.Object) fakeCLIClient {
	t.Helper()

	kube := k8sfake.NewSimpleClientset(secret)
	kube.Resources = []*metav1.APIResourceList{
		{
			GroupVersion: virtualkeysPolicyGVR.GroupVersion().String(),
			APIResources: []metav1.APIResource{
				{Name: virtualkeysPolicyGVR.Resource, Namespaced: true, Kind: "AgentgatewayPolicy"},
			},
		},
	}

	var unstructuredObjs []runtime.Object
	for _, obj := range policyObjects {
		m, err := runtime.DefaultUnstructuredConverter.ToUnstructured(obj)
		if err != nil {
			t.Fatalf("failed to convert fixture to unstructured: %v", err)
		}
		unstructuredObjs = append(unstructuredObjs, &unstructured.Unstructured{Object: m})
	}
	dyn := dynamicfake.NewSimpleDynamicClientWithCustomListKinds(runtime.NewScheme(),
		map[schema.GroupVersionResource]string{virtualkeysPolicyGVR: "AgentgatewayPolicyList"},
		unstructuredObjs...)

	return fakeCLIClient{kube: kube, dyn: dyn}
}

func virtualkeysFixturePolicy(name, namespace string) *agentgateway.AgentgatewayPolicy {
	return &agentgateway.AgentgatewayPolicy{
		TypeMeta:   metav1.TypeMeta{APIVersion: virtualkeysPolicyGVR.GroupVersion().String(), Kind: "AgentgatewayPolicy"},
		ObjectMeta: metav1.ObjectMeta{Name: name, Namespace: namespace},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Traffic: &agentgateway.Traffic{
				APIKeyAuthentication: &agentgateway.APIKeyAuthentication{
					SecretRef: &agentgateway.LocalSecretObjectRef{Name: gwv1.ObjectName("api-key")},
				},
			},
		},
	}
}

func TestVirtualkeysToKeyHashEntryRawKey(t *testing.T) {
	entry, err := virtualkeysToKeyHashEntry([]byte("k-456"))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if entry.Key != "" {
		t.Fatalf("expected raw key to be dropped, got %q", entry.Key)
	}
	if entry.KeyHash != "sha256:"+virtualkeysSHA256Hex("k-456") {
		t.Fatalf("unexpected keyHash: %s", entry.KeyHash)
	}
}

func TestVirtualkeysToKeyHashEntryJSONKey(t *testing.T) {
	entry, err := virtualkeysToKeyHashEntry([]byte(`{"key":"k-123","metadata":{"group":"sales"}}`))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if entry.Key != "" {
		t.Fatalf("expected key to be hashed away, got %q", entry.Key)
	}
	if entry.KeyHash != "sha256:"+virtualkeysSHA256Hex("k-123") {
		t.Fatalf("unexpected keyHash: %s", entry.KeyHash)
	}
	var meta map[string]string
	if err := json.Unmarshal(entry.Metadata, &meta); err != nil {
		t.Fatalf("failed to unmarshal metadata: %v", err)
	}
	if meta["group"] != "sales" {
		t.Fatalf("expected metadata to be preserved, got %v", meta)
	}
}

func TestVirtualkeysToKeyHashEntryExistingHashPreserved(t *testing.T) {
	const hash = "sha256:efa299afb8c12a36e47a790cbbf929caa06d13285950410463fb759af17d0dad"
	entry, err := virtualkeysToKeyHashEntry([]byte(`{"keyHash":"` + hash + `"}`))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if entry.KeyHash != hash {
		t.Fatalf("expected keyHash to be preserved unchanged, got %s", entry.KeyHash)
	}
}

func TestVirtualkeysToKeyHashEntryEmptyValueErrors(t *testing.T) {
	if _, err := virtualkeysToKeyHashEntry([]byte("  ")); err == nil {
		t.Fatal("expected error for empty value")
	}
}

func TestVirtualkeysBuildConfigMap(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data: map[string][]byte{
			"client1": []byte("k-456"),
		},
	}
	labels := map[string]string{virtualkeysMigratedLabelKey: "my-policy"}

	cm, err := virtualkeysBuildConfigMap(secret, "my-policy", labels)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if cm.Name != "api-key-my-policy-configmap" || cm.Namespace != "ns" {
		t.Fatalf("unexpected ConfigMap identity: %s/%s", cm.Namespace, cm.Name)
	}
	if cm.APIVersion != "v1" || cm.Kind != "ConfigMap" {
		t.Fatalf("ConfigMap must set apiVersion/kind to be valid kubectl-apply-able YAML, got %q/%q", cm.APIVersion, cm.Kind)
	}
	if cm.Labels[virtualkeysMigratedLabelKey] != "my-policy" {
		t.Fatalf("expected migration label to be set, got %v", cm.Labels)
	}

	var entry virtualkeysAPIKeyEntry
	if err := json.Unmarshal([]byte(cm.Data["client1"]), &entry); err != nil {
		t.Fatalf("failed to unmarshal ConfigMap entry: %v", err)
	}
	if entry.Key != "" {
		t.Fatalf("ConfigMap entry must not contain a raw key, got %q", entry.Key)
	}
	if entry.KeyHash == "" {
		t.Fatal("expected ConfigMap entry to contain a keyHash")
	}
}

func TestVirtualkeysBuildConfigMapNameScopedPerPolicy(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data:       map[string][]byte{"client1": []byte("k-456")},
	}

	cmA, err := virtualkeysBuildConfigMap(secret, "policy-a", map[string]string{virtualkeysMigratedLabelKey: "policy-a"})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	cmB, err := virtualkeysBuildConfigMap(secret, "policy-b", map[string]string{virtualkeysMigratedLabelKey: "policy-b"})
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if cmA.Name == cmB.Name {
		t.Fatalf("expected distinct ConfigMap names for distinct policies sharing a Secret, got %q for both", cmA.Name)
	}
}

// Without apiVersion/kind on every document, `kubectl apply -f -` rejects the output.
func TestVirtualkeysDryRunOutputIsApplyableYAML(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data:       map[string][]byte{"client1": []byte("k-456")},
	}
	client := virtualkeysFakeClient(t, secret, virtualkeysFixturePolicy("my-policy", "ns"))

	var out, status bytes.Buffer
	if err := runVirtualkeysToConfigMap(context.Background(), &out, &status, client, "ns", "", true); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if got := bytes.Count(out.Bytes(), []byte("apiVersion:")); got != 2 {
		t.Fatalf("expected 2 documents with apiVersion set, got %d\noutput:\n%s", got, out.String())
	}
	if got := bytes.Count(out.Bytes(), []byte("kind:")); got != 2 {
		t.Fatalf("expected 2 documents with kind set, got %d\noutput:\n%s", got, out.String())
	}
	if !bytes.Contains(out.Bytes(), []byte("kind: ConfigMap")) {
		t.Errorf("expected a ConfigMap document, got:\n%s", out.String())
	}
	if !bytes.Contains(out.Bytes(), []byte("kind: AgentgatewayPolicy")) {
		t.Errorf("expected an AgentgatewayPolicy document, got:\n%s", out.String())
	}
}

// A resource without a matching apiKeyAuthentication shape must be skipped, not errored.
func TestVirtualkeysDiscoveryIgnoresNonConformingResources(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data:       map[string][]byte{"client1": []byte("k-456")},
	}
	noAuth := virtualkeysFixturePolicy("no-auth-policy", "ns")
	noAuth.Spec.Traffic.APIKeyAuthentication = nil
	client := virtualkeysFakeClient(t, secret, noAuth)

	var out, status bytes.Buffer
	if err := runVirtualkeysToConfigMap(context.Background(), &out, &status, client, "ns", "", true); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if out.Len() != 0 {
		t.Fatalf("expected no YAML output for a non-conforming resource, got:\n%s", out.String())
	}
}

// Read-time fields on a fetched object must not leak into a manifest meant for GitOps.
func TestVirtualkeysDryRunOutputStripsServerManagedFields(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data:       map[string][]byte{"client1": []byte("k-456")},
	}
	policy := virtualkeysFixturePolicy("my-policy", "ns")
	policy.ObjectMeta.ResourceVersion = "123456"
	policy.ObjectMeta.UID = "abc-123-def-456"
	policy.ObjectMeta.Generation = 3
	policy.ObjectMeta.CreationTimestamp = metav1.NewTime(time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC))
	policy.ObjectMeta.ManagedFields = []metav1.ManagedFieldsEntry{{Manager: "kubectl", Operation: "Update"}}
	client := virtualkeysFakeClient(t, secret, policy)

	var out, status bytes.Buffer
	if err := runVirtualkeysToConfigMap(context.Background(), &out, &status, client, "ns", "", true); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	for _, field := range []string{"resourceVersion", "uid", "generation", "creationTimestamp", "managedFields"} {
		if bytes.Contains(out.Bytes(), []byte(field)) {
			t.Errorf("expected %s to be stripped from the printed manifest, got:\n%s", field, out.String())
		}
	}
	if !bytes.Contains(out.Bytes(), []byte("name: my-policy")) {
		t.Errorf("expected the policy's actual identity to survive stripping, got:\n%s", out.String())
	}
}

// If agentgateway adds a new Kind to the group later, this migration must pick it up via
// discovery alone: found automatically, unrelated fields preserved, output clean and appliable.
func TestVirtualkeysMigratesNewlyAddedKind(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data:       map[string][]byte{"client1": []byte("k-456")},
	}
	kube := k8sfake.NewSimpleClientset(secret)

	gvr := schema.GroupVersionResource{Group: virtualkeysAPIGroup, Version: "v1alpha1", Resource: "widgets"}
	kube.Resources = []*metav1.APIResourceList{
		{
			GroupVersion: gvr.GroupVersion().String(),
			APIResources: []metav1.APIResource{{Name: gvr.Resource, Namespaced: true, Kind: "Widget"}},
		},
	}

	obj := &unstructured.Unstructured{Object: map[string]any{
		"apiVersion": gvr.GroupVersion().String(),
		"kind":       "Widget",
		"metadata": map[string]any{
			"name": "my-widget", "namespace": "ns",
			"resourceVersion": "999", "uid": "widget-uid-1", "generation": int64(2),
		},
		"spec": map[string]any{
			"color": "blue",
			"traffic": map[string]any{
				"apiKeyAuthentication": map[string]any{
					"secretRef": map[string]any{"name": "api-key"},
				},
			},
		},
	}}
	// Seeding via Create (not the constructor) avoids the fake dynamic client's naive
	// Kind->Resource pluralizer, which mishandles words already ending in "s".
	dyn := dynamicfake.NewSimpleDynamicClientWithCustomListKinds(runtime.NewScheme(),
		map[schema.GroupVersionResource]string{gvr: "WidgetList"})
	if _, err := dyn.Resource(gvr).Namespace("ns").Create(context.Background(), obj, metav1.CreateOptions{}); err != nil {
		t.Fatalf("seed create failed: %v", err)
	}
	client := fakeCLIClient{kube: kube, dyn: dyn}

	var out, status bytes.Buffer
	if err := runVirtualkeysToConfigMap(context.Background(), &out, &status, client, "ns", "", true); err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if !bytes.Contains(out.Bytes(), []byte("kind: Widget")) {
		t.Errorf("expected the object's own Kind in the output, got:\n%s", out.String())
	}
	if !bytes.Contains(out.Bytes(), []byte("kind: ConfigMap")) {
		t.Errorf("expected a ConfigMap document, got:\n%s", out.String())
	}
	if !bytes.Contains(out.Bytes(), []byte("color: blue")) {
		t.Errorf("expected a field this migration doesn't touch to survive untouched, got:\n%s", out.String())
	}
	for _, field := range []string{"resourceVersion", "uid", "generation"} {
		if bytes.Contains(out.Bytes(), []byte(field)) {
			t.Errorf("expected %s to be stripped, got:\n%s", field, out.String())
		}
	}
}

func virtualkeysSHA256Hex(s string) string {
	entry, err := virtualkeysToKeyHashEntry([]byte(s))
	if err != nil {
		panic(err)
	}
	const prefix = "sha256:"
	return entry.KeyHash[len(prefix):]
}
