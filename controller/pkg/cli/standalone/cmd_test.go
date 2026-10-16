package standalone

import (
	"os"
	"regexp"
	"slices"
	"testing"
)

func TestKindsMatchServer(t *testing.T) {
	src, err := os.ReadFile("../../../../crates/agentgateway/src/config_store.rs")
	if err != nil {
		t.Fatal(err)
	}
	enum := regexp.MustCompile(`(?s)pub enum ConfigResourceKind \{(.*?)\n\}`).FindSubmatch(src)
	if enum == nil {
		t.Fatal("ConfigResourceKind enum not found")
	}
	var want []string
	for _, m := range regexp.MustCompile(`#\[serde\(rename = "([^"]+)"\)\]`).FindAllSubmatch(enum[1], -1) {
		want = append(want, string(m[1]))
	}
	got := slices.Clone(kinds)
	slices.Sort(want)
	slices.Sort(got)
	if !slices.Equal(got, want) {
		t.Fatalf("kinds out of sync with ConfigResourceKind in config_store.rs:\ngot:  %v\nwant: %v", got, want)
	}
}
