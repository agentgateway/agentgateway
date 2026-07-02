package virtualkey

import (
	"context"
	"regexp"
	"strings"
	"testing"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes/fake"
)

func TestGenerateAPIKeyFormat(t *testing.T) {
	key, err := generateAPIKey("Alice Smith")
	if err != nil {
		t.Fatal(err)
	}
	if !regexp.MustCompile(`^sk-alice-smith-[a-z0-9]{18}$`).MatchString(key) {
		t.Fatalf("generated key has unexpected format: %s", key)
	}
}

func TestGenerateAPIKeyFallbackLabel(t *testing.T) {
	key, err := generateAPIKey("")
	if err != nil {
		t.Fatal(err)
	}
	if !regexp.MustCompile(`^sk-key-[a-z0-9]{18}$`).MatchString(key) {
		t.Fatalf("generated key has unexpected fallback format: %s", key)
	}
}

func TestGenerateAPIKeyUnique(t *testing.T) {
	seen := map[string]struct{}{}
	for i := range 100 {
		key, err := generateAPIKey("alice")
		if err != nil {
			t.Fatal(err)
		}
		if _, ok := seen[key]; ok {
			t.Fatalf("generated duplicate key at iteration %d", i)
		}
		seen[key] = struct{}{}
	}
}

func TestSanitizeLabel(t *testing.T) {
	tests := map[string]string{
		"":                                      "key",
		"Sales/Alice!!":                         "sales-alice",
		"---":                                   "key",
		"this-label-is-definitely-way-too-long": "this-label-is-definitely",
	}
	for input, want := range tests {
		if got := sanitizeLabel(input); got != want {
			t.Fatalf("sanitizeLabel(%q) = %q, want %q", input, got, want)
		}
	}
}

func TestLoadExistingKeysParsesRawAndJSONSecretValues(t *testing.T) {
	client := fake.NewSimpleClientset(&corev1.Secret{
		ObjectMeta: metav1.ObjectMeta{Name: "keys", Namespace: "default"},
		Data: map[string][]byte{
			"raw":  []byte("k-raw"),
			"json": []byte(`{"key":"k-json","metadata":{"id":"vk-json"}}`),
		},
	})

	keys, err := loadExistingKeys(context.Background(), client.CoreV1().Secrets("default"), "keys", "")
	if err != nil {
		t.Fatal(err)
	}
	got := map[string]string{}
	for _, key := range keys {
		got[key.Entry] = key.Key
	}
	if got["raw"] != "k-raw" || got["json"] != "k-json" {
		t.Fatalf("unexpected parsed keys: %#v", got)
	}
}

func TestCheckCollisionsDoesNotLeakRawKey(t *testing.T) {
	err := checkCollisions(map[int]string{7: "k-secret-value"}, []existingKey{{
		Secret: "existing",
		Entry:  "alice",
		Key:    "k-secret-value",
	}})
	if err == nil {
		t.Fatal("expected collision error")
	}
	msg := err.Error()
	if !strings.Contains(msg, "row 7") || !strings.Contains(msg, "existing") || !strings.Contains(msg, "alice") {
		t.Fatalf("collision error missing expected context: %s", msg)
	}
	if strings.Contains(msg, "k-secret-value") {
		t.Fatalf("collision error leaked raw key: %s", msg)
	}
}
