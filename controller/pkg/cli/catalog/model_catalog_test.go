package catalog

import (
	"slices"
	"testing"
)

func TestMergeFromKeepsBaseRatesAndUnionsTags(t *testing.T) {
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

	base.mergeFrom(overlay)

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

func TestMergeFromInsertsNewProviderAndOverlaysEmptyRates(t *testing.T) {
	base := &ModelCatalog{Providers: map[string]Provider{}}
	// Overlay fills fields the base leaves empty.
	base.mergeFrom(&ModelCatalog{Providers: map[string]Provider{
		"aws.bedrock": {Models: map[string]Model{"m": {Rates: Rates{Input: "1"}, Tags: []string{"a"}}}},
	}})
	base.mergeFrom(&ModelCatalog{Providers: map[string]Provider{
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

func TestMergeFromReplacesTiersWhenOverlayHasThem(t *testing.T) {
	base := &ModelCatalog{Providers: map[string]Provider{
		"p": {Models: map[string]Model{"m": {Tiers: []Tier{{ContextOver: 1000, Rates: Rates{Input: "1"}}}}}},
	}}
	base.mergeFrom(&ModelCatalog{Providers: map[string]Provider{
		"p": {Models: map[string]Model{"m": {Tiers: []Tier{{ContextOver: 2000, Rates: Rates{Input: "2"}}}}}},
	}})
	tiers := base.Providers["p"].Models["m"].Tiers
	if len(tiers) != 1 || tiers[0].ContextOver != 2000 {
		t.Errorf("tiers = %+v, want single tier ContextOver=2000", tiers)
	}
}
