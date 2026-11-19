package standalone

import (
	"bytes"
	"encoding/json"
	"net/http"
	"os"
	"regexp"
	"slices"
	"testing"
	"testing/fstest"
)

func TestGetResourceSource(t *testing.T) {
	c := &client{
		address: "file://",
		http: &http.Client{Transport: http.NewFileTransportFS(fstest.MapFS{
			"api/config/resources": {Data: []byte(`{"resources":[
				{"kind":"llm.provider","id":"file-provider","source":"file","value":{"name":"file-provider"}},
				{"kind":"llm.provider","id":"db-provider","source":"database","value":{"name":"db-provider"}},
				{"kind":"llm.provider","id":"legacy-provider","revision":1,"value":{"name":"legacy-provider"}}
			]}`)},
		})},
	}

	for _, tc := range []struct {
		name        string
		flags       []string
		wantNames   []string
		wantSources []string
	}{
		{
			name:        "all by default",
			wantNames:   []string{"file-provider", "db-provider", "legacy-provider"},
			wantSources: []string{"file", "database", "database"},
		},
		{
			name:        "file only",
			flags:       []string{"--source", "file"},
			wantNames:   []string{"file-provider"},
			wantSources: []string{"file"},
		},
		{
			name:        "database export without metadata",
			flags:       []string{"--source", "database", "--no-metadata"},
			wantNames:   []string{"db-provider", "legacy-provider"},
			wantSources: []string{"", ""},
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var out bytes.Buffer
			cmd := getCommand(c)
			cmd.SetOut(&out)
			cmd.SetArgs(append([]string{"all", "-o", "json"}, tc.flags...))
			if err := cmd.Execute(); err != nil {
				t.Fatal(err)
			}
			var resources []resource
			if err := json.Unmarshal(out.Bytes(), &resources); err != nil {
				t.Fatal(err)
			}
			var names, sources []string
			for _, r := range resources {
				names = append(names, resourceName(r))
				sources = append(sources, r.Source)
			}
			if !slices.Equal(names, tc.wantNames) || !slices.Equal(sources, tc.wantSources) {
				t.Fatalf("unexpected resources: %s", out.String())
			}
		})
	}
}

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
		// session.key is internal to the server and never exposed through the API.
		if string(m[1]) == "session.key" {
			continue
		}
		want = append(want, string(m[1]))
	}
	got := slices.Clone(kinds)
	slices.Sort(want)
	slices.Sort(got)
	if !slices.Equal(got, want) {
		t.Fatalf("kinds out of sync with ConfigResourceKind in config_store.rs:\ngot:  %v\nwant: %v", got, want)
	}
}
