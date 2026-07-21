package virtualkeys

import (
	"encoding/json"
	"testing"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

func TestToKeyHashEntryRawKey(t *testing.T) {
	entry, err := toKeyHashEntry([]byte("k-456"))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if entry.Key != "" {
		t.Fatalf("expected raw key to be dropped, got %q", entry.Key)
	}
	if entry.KeyHash != "sha256:"+sha256Hex("k-456") {
		t.Fatalf("unexpected keyHash: %s", entry.KeyHash)
	}
}

func TestToKeyHashEntryJSONKey(t *testing.T) {
	entry, err := toKeyHashEntry([]byte(`{"key":"k-123","metadata":{"group":"sales"}}`))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if entry.Key != "" {
		t.Fatalf("expected key to be hashed away, got %q", entry.Key)
	}
	if entry.KeyHash != "sha256:"+sha256Hex("k-123") {
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

func TestToKeyHashEntryExistingHashPreserved(t *testing.T) {
	const hash = "sha256:efa299afb8c12a36e47a790cbbf929caa06d13285950410463fb759af17d0dad"
	entry, err := toKeyHashEntry([]byte(`{"keyHash":"` + hash + `"}`))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if entry.KeyHash != hash {
		t.Fatalf("expected keyHash to be preserved unchanged, got %s", entry.KeyHash)
	}
}

func TestToKeyHashEntryEmptyValueErrors(t *testing.T) {
	if _, err := toKeyHashEntry([]byte("  ")); err == nil {
		t.Fatal("expected error for empty value")
	}
}

func TestBuildConfigMap(t *testing.T) {
	secret := &corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "api-key", Namespace: "ns"},
		Data: map[string][]byte{
			"client1": []byte("k-456"),
		},
	}
	labels := map[string]string{migratedLabelKey: "my-policy"}

	cm, err := buildConfigMap(secret, labels)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if cm.Name != "api-key-configmap" || cm.Namespace != "ns" {
		t.Fatalf("unexpected ConfigMap identity: %s/%s", cm.Namespace, cm.Name)
	}
	if cm.Labels[migratedLabelKey] != "my-policy" {
		t.Fatalf("expected migration label to be set, got %v", cm.Labels)
	}

	var entry apiKeyEntry
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

func sha256Hex(s string) string {
	entry, err := toKeyHashEntry([]byte(s))
	if err != nil {
		panic(err)
	}
	const prefix = "sha256:"
	return entry.KeyHash[len(prefix):]
}
