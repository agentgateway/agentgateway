package catalog

import (
	"reflect"
	"slices"
	"testing"

	"sigs.k8s.io/yaml"
)

func TestOverlayWithKeepsBaseRatesAndUnionsTags(t *testing.T) {
	// models.dev-style base: rates, no tags. bedrock-style overlay: tags, no rates.
	base := &ModelCatalog{Providers: map[string]Provider{
		bedrockProviderID: {Models: map[string]Model{
			"anthropic.claude-opus-4-8": {Rates: Rates{Input: "3", Output: "15"}, Tags: []string{"existing"}},
		}},
	}}
	overlay := &ModelCatalog{Providers: map[string]Provider{
		bedrockProviderID: {Models: map[string]Model{
			// present in base: rates must be kept, tags unioned
			"anthropic.claude-opus-4-8": {Tags: []string{mantleTag, runtimeTag}},
			// absent from base: inserted as-is (tags only)
			"openai.gpt-oss-120b": {Tags: []string{mantleTag}},
		}},
	}}

	base.overlayWith(overlay)

	got := base.Providers[bedrockProviderID].Models
	opus := got["anthropic.claude-opus-4-8"]
	if opus.Rates.Input != "3" || opus.Rates.Output != "15" {
		t.Errorf("base rates not preserved: %+v", opus.Rates)
	}
	if want := []string{"existing", mantleTag, runtimeTag}; !slices.Equal(opus.Tags, want) {
		t.Errorf("opus tags = %v, want %v", opus.Tags, want)
	}
	gpt := got["openai.gpt-oss-120b"]
	if !slices.Equal(gpt.Tags, []string{mantleTag}) || !gpt.Rates.IsZero() {
		t.Errorf("gpt merged = %+v, want tags [mantle] and zero rates", gpt)
	}
}

func TestOverlayWithInsertsNewProviderAndFillsEmptyRates(t *testing.T) {
	base := &ModelCatalog{Providers: map[string]Provider{}}
	// Overlay fills fields the base leaves empty.
	base.overlayWith(&ModelCatalog{Providers: map[string]Provider{
		"aws.bedrock": {Models: map[string]Model{"m": {Rates: Rates{Input: "1"}, Tags: []string{"a"}}}},
	}})
	base.overlayWith(&ModelCatalog{Providers: map[string]Provider{
		"aws.bedrock": {Models: map[string]Model{"m": {Rates: Rates{Output: "2"}, Tags: []string{"b"}}}},
	}})

	m := base.Providers["aws.bedrock"].Models["m"]
	if m.Rates.Input != "1" || m.Rates.Output != "2" {
		t.Errorf("overlay rates = %+v, want input=1 output=2", m.Rates)
	}
	if want := []string{"a", "b"}; !slices.Equal(m.Tags, want) {
		t.Errorf("tags = %v, want %v", m.Tags, want)
	}
}

func TestOverlayWithReplacesTiersWhenOverlayHasThem(t *testing.T) {
	base := &ModelCatalog{Providers: map[string]Provider{
		"p": {Models: map[string]Model{"m": {Tiers: []Tier{{ContextOver: 1000, Rates: Rates{Input: "1"}}}}}},
	}}
	base.overlayWith(&ModelCatalog{Providers: map[string]Provider{
		"p": {Models: map[string]Model{"m": {Tiers: []Tier{{ContextOver: 2000, Rates: Rates{Input: "2"}}}}}},
	}})
	tiers := base.Providers["p"].Models["m"].Tiers
	if len(tiers) != 1 || tiers[0].ContextOver != 2000 {
		t.Errorf("tiers = %+v, want single tier ContextOver=2000", tiers)
	}
}

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
