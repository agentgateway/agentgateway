package virtualkey

import (
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"testing"
)

func TestImportCSVBuildsSecretManifest(t *testing.T) {
	input := `entry,key,id,metadata.group,metadata.tier,metadata.owner
alice,k-live-alice-001,vk-sales-alice,sales,premium,alice
bob,k-live-bob-001,vk-sales-bob,sales,standard,bob
`
	result, err := importCSV(strings.NewReader(input), importOptions{
		SecretName: "agw-virtual-keys",
		Namespace:  "default",
		Labels:     map[string]string{"team": "platform"},
	})
	if err != nil {
		t.Fatal(err)
	}
	if result.Secret.APIVersion != "v1" || result.Secret.Kind != "Secret" || result.Secret.Type != "Opaque" {
		t.Fatalf("unexpected Secret metadata: %#v", result.Secret)
	}
	if result.Secret.Metadata.Name != "agw-virtual-keys" || result.Secret.Metadata.Namespace != "default" {
		t.Fatalf("unexpected manifest metadata: %#v", result.Secret.Metadata)
	}
	if result.Secret.Metadata.Labels[virtualKeysLabel] != "true" || result.Secret.Metadata.Labels["team"] != "platform" {
		t.Fatalf("unexpected labels: %#v", result.Secret.Metadata.Labels)
	}
	if len(result.Secrets) != 1 {
		t.Fatalf("expected normal import to stay in one Secret, got %d", len(result.Secrets))
	}

	entry := decodeEntry(t, result.Secret.StringData["alice"])
	if entry.Key != "k-live-alice-001" {
		t.Fatal("expected imported key to be preserved")
	}
	if entry.Metadata["id"] != "vk-sales-alice" || entry.Metadata["group"] != "sales" || entry.Metadata["tier"] != "premium" {
		t.Fatalf("unexpected metadata: %#v", entry.Metadata)
	}
	if _, ok := entry.Metadata["owner"]; !ok {
		t.Fatalf("expected arbitrary metadata column to be preserved: %#v", entry.Metadata)
	}
}

func TestImportCSVGeneratesMissingKeyAndOmitsEmptyMetadata(t *testing.T) {
	input := `entry,key,id,metadata.group,metadata.empty
charlie,,vk-sales-charlie,sales,
`
	result, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys"})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.GeneratedRows) != 1 || result.GeneratedRows[0] != 2 {
		t.Fatalf("unexpected generated rows: %#v", result.GeneratedRows)
	}
	entry := decodeEntry(t, result.Secret.StringData["charlie"])
	if !regexp.MustCompile(`^sk-charlie-[a-z0-9]{18}$`).MatchString(entry.Key) {
		t.Fatalf("generated key has unexpected format: %s", entry.Key)
	}
	if _, ok := entry.Metadata["empty"]; ok {
		t.Fatalf("expected empty metadata value to be omitted: %#v", entry.Metadata)
	}
}

func TestImportCSVSplitsLargeImports(t *testing.T) {
	input := largeImportCSV(4, 520)
	result, err := importCSV(strings.NewReader(input), importOptions{
		SecretName:              "agw-virtual-keys",
		Namespace:               "default",
		Labels:                  map[string]string{"team": "platform"},
		MaxSerializedSecretSize: 1350,
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(result.Secrets) != 4 {
		t.Fatalf("expected one entry per split Secret, got %d", len(result.Secrets))
	}
	wantNames := []string{"agw-virtual-keys", "agw-virtual-keys-0002", "agw-virtual-keys-0003", "agw-virtual-keys-0004"}
	for i, secret := range result.Secrets {
		if secret.Metadata.Name != wantNames[i] {
			t.Fatalf("secret %d name = %q, want %q", i, secret.Metadata.Name, wantNames[i])
		}
		if secret.Metadata.Namespace != "default" {
			t.Fatalf("secret %d namespace = %q", i, secret.Metadata.Namespace)
		}
		if secret.Metadata.Labels[virtualKeysLabel] != "true" || secret.Metadata.Labels["team"] != "platform" {
			t.Fatalf("secret %d labels = %#v", i, secret.Metadata.Labels)
		}
		entryName := wantEntryName(i + 1)
		if _, ok := secret.StringData[entryName]; !ok {
			t.Fatalf("secret %d missing deterministic entry %q: %#v", i, entryName, secret.StringData)
		}
		size, err := serializedSecretSize(secret)
		if err != nil {
			t.Fatal(err)
		}
		if size > 1350 {
			t.Fatalf("secret %q serialized size %d exceeds threshold", secret.Metadata.Name, size)
		}
	}

	manifest, ok := result.Manifest().(manifestList)
	if !ok {
		t.Fatalf("expected split import to print a List, got %T", result.Manifest())
	}
	if len(manifest.Items) != len(result.Secrets) {
		t.Fatalf("manifest items = %d, want %d", len(manifest.Items), len(result.Secrets))
	}
}

func TestImportCSVRejectsOverlargeSingleRow(t *testing.T) {
	input := largeImportCSV(1, 800)
	_, err := importCSV(strings.NewReader(input), importOptions{
		SecretName:              "keys",
		MaxSerializedSecretSize: 300,
	})
	if err == nil {
		t.Fatal("expected overlarge single row error")
	}
	if !strings.Contains(err.Error(), `row 2 Secret entry "entry-0001" exceeds max serialized Secret size 300 bytes`) {
		t.Fatalf("unexpected error: %v", err)
	}
	if strings.Contains(err.Error(), strings.Repeat("x", 20)) {
		t.Fatalf("error leaked metadata payload: %v", err)
	}
}

func TestImportCSVRejectsSplitSecretNameTooLong(t *testing.T) {
	input := largeImportCSV(2, 520)
	_, err := importCSV(strings.NewReader(input), importOptions{
		SecretName:              strings.Repeat("a", maxSecretNameLength-4),
		MaxSerializedSecretSize: 1350,
	})
	if err == nil {
		t.Fatal("expected long split name error")
	}
	if !strings.Contains(err.Error(), "is too long to split") || !strings.Contains(err.Error(), "exceeds 253 characters") {
		t.Fatalf("unexpected error: %v", err)
	}
}

func TestImportCSVWarnsForMissingIDAndDuplicates(t *testing.T) {
	input := `entry,key,id,metadata.group
a,k-shared,,support
b,k-shared,,support
`
	result, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys"})
	if err != nil {
		t.Fatal(err)
	}
	got := strings.Join(result.Warnings, "\n")
	for _, want := range []string{"row 2 has no id", "row 3 has no id", "duplicate API key value at rows 2, 3"} {
		if !strings.Contains(got, want) {
			t.Fatalf("warnings missing %q: %s", want, got)
		}
	}
	if strings.Contains(got, "k-shared") {
		t.Fatalf("warning leaked raw key: %s", got)
	}
}

func TestImportCSVStrictTreatsWarningsAsErrors(t *testing.T) {
	input := `entry,key,id
a,k-a,
`
	_, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys", Strict: true})
	if err == nil {
		t.Fatal("expected strict mode error")
	}
	if !strings.Contains(err.Error(), "strict validation failed") || strings.Contains(err.Error(), "k-a") {
		t.Fatalf("unexpected strict error: %v", err)
	}
}

func TestImportCSVRejectsDuplicateEntryNames(t *testing.T) {
	input := `entry,key,id
alice,k-a,vk-a
alice,k-b,vk-b
`
	_, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys"})
	if err == nil {
		t.Fatal("expected duplicate entry error")
	}
	if strings.Contains(err.Error(), "k-a") || strings.Contains(err.Error(), "k-b") {
		t.Fatalf("duplicate entry error leaked raw key: %v", err)
	}
}

func TestImportCSVRejectsReservedIDCharactersByDefault(t *testing.T) {
	input := "entry,key,id\nalice,k-a,vk|alice\n"
	_, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys"})
	if err == nil || !strings.Contains(err.Error(), "reserved special characters") {
		t.Fatalf("expected reserved character error, got %v", err)
	}
}

func TestImportCSVEscapesReservedIDCharacters(t *testing.T) {
	input := "entry,key,id\nalice,k-a,vk|alice\n"
	result, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys", EscapeIDs: true})
	if err != nil {
		t.Fatal(err)
	}
	entry := decodeEntry(t, result.Secret.StringData["alice"])
	if entry.Metadata["id"] != `vk\|alice` {
		t.Fatalf("expected escaped id, got %#v", entry.Metadata["id"])
	}
}

func TestImportCSVEscapesReservedIDValue(t *testing.T) {
	input := "entry,key,id\nalice,k-a,<missing>\n"
	result, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys", EscapeIDs: true})
	if err != nil {
		t.Fatal(err)
	}
	entry := decodeEntry(t, result.Secret.StringData["alice"])
	if entry.Metadata["id"] != `\<missing>` {
		t.Fatalf("expected escaped id, got %#v", entry.Metadata["id"])
	}
}

func TestImportCSVRejectsConflictingMetadataID(t *testing.T) {
	input := "entry,key,id,metadata.id\nalice,k-a,vk-a,vk-b\n"
	_, err := importCSV(strings.NewReader(input), importOptions{SecretName: "keys"})
	if err == nil || !strings.Contains(err.Error(), "conflicting id") {
		t.Fatalf("expected conflicting id error, got %v", err)
	}
}

func decodeEntry(t *testing.T, value string) apiKeyEntry {
	t.Helper()
	var entry apiKeyEntry
	if err := json.Unmarshal([]byte(value), &entry); err != nil {
		t.Fatal(err)
	}
	return entry
}

func largeImportCSV(rows, metadataSize int) string {
	var b strings.Builder
	b.WriteString("entry,key,id,metadata.notes\n")
	for i := 1; i <= rows; i++ {
		fmt.Fprintf(&b, "entry-%04d,k-%04d,%s,%s\n", i, i, wantEntryID(i), strings.Repeat("x", metadataSize))
	}
	return b.String()
}

func wantEntryID(i int) string {
	return fmt.Sprintf("vk-%04d", i)
}

func wantEntryName(i int) string {
	return fmt.Sprintf("entry-%04d", i)
}
