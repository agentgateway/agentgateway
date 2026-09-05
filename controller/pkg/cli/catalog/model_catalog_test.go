package catalog

import (
	"reflect"
	"testing"

	"sigs.k8s.io/yaml"
)

func TestOverlayCatalog(t *testing.T) {
	base := ModelCatalog{Providers: map[string]Provider{
		"openai": {Models: map[string]Model{
			"existing": {Rates: Rates{Input: "1", Output: "2"}},
		}},
	}}
	var overlay ModelCatalog
	if err := yaml.UnmarshalStrict([]byte(`
providers:
  openai:
    models:
      existing:
        rates:
          output: "3"
      added:
        rates:
          input: "4"
`), &overlay); err != nil {
		t.Fatal(err)
	}

	base.overlayWith(&overlay)

	want := ModelCatalog{Providers: map[string]Provider{
		"openai": {Models: map[string]Model{
			"existing": {Rates: Rates{Input: "1", Output: "3"}},
			"added":    {Rates: Rates{Input: "4"}},
		}},
	}}
	if !reflect.DeepEqual(base, want) {
		t.Fatalf("merged catalog = %#v, want %#v", base, want)
	}
}

func TestOverlayCatalogWildcards(t *testing.T) {
	base := ModelCatalog{Providers: map[string]Provider{
		"anthropic": {Models: map[string]Model{
			"claude-opus-4-6":   {Tags: []string{"legacy_thinking"}},
			"claude-sonnet-4.6": {Tags: []string{"adaptive_thinking"}},
			"claude-opus-4-5":   {Tags: []string{"legacy_thinking"}},
		}},
		"aws.bedrock": {Models: map[string]Model{
			"us/anthropic.claude-opus-4-6-v1": {Tags: []string{"legacy_thinking"}},
		}},
	}}
	var overlay ModelCatalog
	if err := yaml.UnmarshalStrict([]byte(`
providers:
  "*":
    models:
      "*opus-4-6*":
        tags: [adaptive_thinking]
      "*sonnet-4.6*":
        tags: [adaptive_thinking]
`), &overlay); err != nil {
		t.Fatal(err)
	}

	base.overlayWith(&overlay)

	if got := base.Providers["anthropic"].Models["claude-opus-4-6"].Tags; !reflect.DeepEqual(got, []string{"legacy_thinking", "adaptive_thinking"}) {
		t.Fatalf("dash model tags = %v", got)
	}
	if got := base.Providers["anthropic"].Models["claude-sonnet-4.6"].Tags; !reflect.DeepEqual(got, []string{"adaptive_thinking"}) {
		t.Fatalf("duplicate tag was not removed: %v", got)
	}
	if got := base.Providers["aws.bedrock"].Models["us/anthropic.claude-opus-4-6-v1"].Tags; !reflect.DeepEqual(got, []string{"legacy_thinking", "adaptive_thinking"}) {
		t.Fatalf("slash-containing model tags = %v", got)
	}
	if got := base.Providers["anthropic"].Models["claude-opus-4-5"].Tags; !reflect.DeepEqual(got, []string{"legacy_thinking"}) {
		t.Fatalf("unmatched model tags = %v", got)
	}
	if _, found := base.Providers["anthropic"].Models["*opus-4-6*"]; found {
		t.Fatal("wildcard was emitted as a literal model")
	}
}
