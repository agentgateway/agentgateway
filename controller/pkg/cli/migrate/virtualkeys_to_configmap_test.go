package migrate

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"testing"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	k8sfake "k8s.io/client-go/kubernetes/fake"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	gatewayapiclient "sigs.k8s.io/gateway-api/pkg/client/clientset/versioned"

	agentgateway "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	agentgatewayclient "github.com/agentgateway/agentgateway/controller/pkg/client/clientset/versioned"
	agentgatewayfake "github.com/agentgateway/agentgateway/controller/pkg/client/clientset/versioned/fake"
)

// fakeCLIClient is a minimal kubeutil.CLIClient backed by fake clientsets, for
// exercising the dry-run YAML output end to end.
type fakeCLIClient struct {
	kube kubernetes.Interface
	agw  agentgatewayclient.Interface
}

func (f fakeCLIClient) Kube() kubernetes.Interface                 { return f.kube }
func (f fakeCLIClient) GatewayAPI() gatewayapiclient.Interface     { return nil }
func (f fakeCLIClient) Agentgateway() agentgatewayclient.Interface { return f.agw }
func (f fakeCLIClient) AgentgatewayRequest(context.Context, string, string, string, string, int) ([]byte, error) {
	return nil, fmt.Errorf("not implemented")
}
func (f fakeCLIClient) NewPortForwarder(string, string, string, int, int) (kubeutil.PortForwarder, error) {
	return nil, fmt.Errorf("not implemented")
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

// TestVirtualkeysDryRunOutputIsApplyableYAML exercises the full dry-run path
// against fake clientsets and checks that every printed document sets
// apiVersion/kind - without them, `kubectl apply -f -` rejects the output.
func TestVirtualkeysDryRunOutputIsApplyableYAML(t *testing.T) {
	kube := k8sfake.NewSimpleClientset(&corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data:       map[string][]byte{"client1": []byte("k-456")},
	})
	agw := agentgatewayfake.NewSimpleClientset(&agentgateway.AgentgatewayPolicy{
		ObjectMeta: metav1.ObjectMeta{Name: "my-policy", Namespace: "ns"},
		Spec: agentgateway.AgentgatewayPolicySpec{
			Traffic: &agentgateway.Traffic{
				APIKeyAuthentication: &agentgateway.APIKeyAuthentication{
					SecretRef: &agentgateway.LocalSecretObjectRef{Name: gwv1.ObjectName("api-key")},
				},
			},
		},
	})
	client := fakeCLIClient{kube: kube, agw: agw}

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

func virtualkeysSHA256Hex(s string) string {
	entry, err := virtualkeysToKeyHashEntry([]byte(s))
	if err != nil {
		panic(err)
	}
	const prefix = "sha256:"
	return entry.KeyHash[len(prefix):]
}
